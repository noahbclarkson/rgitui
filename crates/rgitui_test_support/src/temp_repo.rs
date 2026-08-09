//! A throwaway on-disk git repository for tests.

use std::cell::Cell;
use std::path::Path;

use git2::{Commit, Oid, Repository, RepositoryInitOptions, Signature, Time};
use tempfile::TempDir;

/// Wall clock the first signature is stamped with; each later signature adds a
/// second. Fixed so commit OIDs and ordering never depend on when the suite ran.
const FIRST_COMMIT_TIME: i64 = 1_700_000_000; // 2023-11-14T22:13:20Z

/// A git repository in a temporary directory, deleted on drop.
///
/// Nearly every git test needs the same three lines — make a temp dir, init a
/// repo in it, configure an identity so committing works — and then has to keep
/// the [`TempDir`] alive by hand: bind it to `_dir` and the directory is
/// deleted before the test body runs. `TempRepo` owns the directory, so there
/// is nothing to hold on to and nothing to get wrong.
///
/// Commits are authored by [`TempRepo::AUTHOR_NAME`] on a deterministic,
/// monotonically increasing clock, so tests never depend on the machine's git
/// config or clock. `HEAD` starts on [`TempRepo::DEFAULT_BRANCH`] regardless of
/// `init.defaultBranch`.
///
/// ```no_run
/// # use rgitui_test_support::TempRepo;
/// let repo = TempRepo::init();
/// let first = repo.commit_file("hello.txt", "line1\n", "add hello");
/// repo.branch("feature");
/// repo.write_file("hello.txt", "line1\nline2\n"); // unstaged edit
///
/// let head = git2::Repository::open(repo.path()).unwrap();
/// assert_eq!(head.head().unwrap().peel_to_commit().unwrap().id(), first);
/// ```
pub struct TempRepo {
    // Declared before `dir` so libgit2 closes its file handles before the
    // directory is removed; Windows will not delete files that are still open.
    repo: Repository,
    dir: TempDir,
    signatures_issued: Cell<i64>,
}

impl TempRepo {
    /// Name every commit is authored and committed by, unless overridden with
    /// [`TempRepo::commit_file_as`].
    pub const AUTHOR_NAME: &'static str = "Test User";
    /// Email that goes with [`TempRepo::AUTHOR_NAME`].
    pub const AUTHOR_EMAIL: &'static str = "test@example.com";
    /// Branch `HEAD` points at from the start.
    pub const DEFAULT_BRANCH: &'static str = "main";

    /// Creates an empty repository with an identity configured and `HEAD` on
    /// [`TempRepo::DEFAULT_BRANCH`] (still unborn — there are no commits yet).
    pub fn init() -> Self {
        let dir = TempDir::new().expect("failed to create temp dir");
        let mut options = RepositoryInitOptions::new();
        options.initial_head(Self::DEFAULT_BRANCH);
        let repo = Repository::init_opts(dir.path(), &options).expect("failed to init repo");

        // Some code under test asks libgit2 for the default signature
        // (`repo.signature()`), which reads this config rather than our
        // `Signature`, so both have to be set.
        let mut config = repo.config().expect("failed to open repo config");
        config
            .set_str("user.name", Self::AUTHOR_NAME)
            .expect("failed to set user.name");
        config
            .set_str("user.email", Self::AUTHOR_EMAIL)
            .expect("failed to set user.email");
        // Keep checkout byte-exact so content assertions hold wherever the suite
        // runs; the machine's global core.autocrlf would otherwise rewrite line
        // endings on Windows.
        config
            .set_bool("core.autocrlf", false)
            .expect("failed to set core.autocrlf");
        drop(config);

        Self {
            repo,
            dir,
            signatures_issued: Cell::new(0),
        }
    }

    /// Creates a repository with `n` commits messaged `commit 0`..`commit n-1`
    /// on [`TempRepo::DEFAULT_BRANCH`], each with an empty tree.
    ///
    /// Use this when a test cares about history shape — paging, reflog, reset
    /// targets — and not about file contents. Reach for [`TempRepo::init`] plus
    /// [`TempRepo::commit_file`] when the trees matter.
    pub fn with_commits(n: usize) -> Self {
        let repo = Self::init();
        for i in 0..n {
            repo.commit(&format!("commit {}", i));
        }
        repo
    }

    /// The repository's working directory, which is also the temp directory.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// The open repository, for assertions and states the builders don't cover.
    pub fn repo(&self) -> &Repository {
        &self.repo
    }

    /// The OID `HEAD` resolves to. Panics if `HEAD` is unborn.
    pub fn head_oid(&self) -> Oid {
        self.repo
            .head()
            .expect("HEAD should exist")
            .peel_to_commit()
            .expect("HEAD should point at a commit")
            .id()
    }

    /// Returns a signature at the next tick of this fixture's deterministic
    /// clock.
    ///
    /// Each call advances the clock, including calls used for non-commit Git
    /// objects such as annotated tags. Prefer [`TempRepo::commit_tree`] when
    /// constructing commits with custom trees, parents, or refs so the commit
    /// and clock update cannot get out of sync.
    pub fn signature(&self) -> Signature<'static> {
        self.signature_for(Self::AUTHOR_NAME, Self::AUTHOR_EMAIL)
    }

    /// Writes `contents` to `relative_path` in the working tree, creating parent
    /// directories. The change is left unstaged.
    pub fn write_file(&self, relative_path: impl AsRef<Path>, contents: &str) {
        let path = self.path().join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("failed to create parent dirs");
        }
        std::fs::write(&path, contents).expect("failed to write file");
    }

    /// Stages `relative_path`, whose path is interpreted relative to the
    /// working tree.
    pub fn stage(&self, relative_path: impl AsRef<Path>) {
        let mut index = self.repo.index().expect("failed to open index");
        index
            .add_path(relative_path.as_ref())
            .expect("failed to stage path");
        index.write().expect("failed to write index");
    }

    /// Commits whatever is staged onto `HEAD` and returns the new commit's OID.
    pub fn commit(&self, message: &str) -> Oid {
        let tree_oid = self
            .repo
            .index()
            .expect("failed to open index")
            .write_tree()
            .expect("failed to write tree");

        // An unborn HEAD has no commit to parent onto; that is the root commit.
        let parents = self
            .repo
            .head()
            .ok()
            .and_then(|head| head.peel_to_commit().ok())
            .map(|parent| vec![parent.id()])
            .unwrap_or_default();

        self.commit_tree(Some("HEAD"), message, tree_oid, &parents)
    }

    /// Creates a commit from an existing tree and explicit parent OIDs.
    ///
    /// `update_ref` accepts the same values as [`Repository::commit`], including
    /// `Some("HEAD")`, an explicit ref such as `Some("refs/heads/feature")`,
    /// or `None` for an unreachable commit. The fixture supplies a fresh
    /// deterministic signature for every call.
    pub fn commit_tree(
        &self,
        update_ref: Option<&str>,
        message: &str,
        tree_oid: Oid,
        parent_oids: &[Oid],
    ) -> Oid {
        let signature = self.signature();
        self.commit_tree_signed_by(update_ref, &signature, message, tree_oid, parent_oids)
    }

    /// Writes, stages, and commits one file in a single step.
    pub fn commit_file(
        &self,
        relative_path: impl AsRef<Path>,
        contents: &str,
        message: &str,
    ) -> Oid {
        self.commit_file_as(
            Self::AUTHOR_NAME,
            Self::AUTHOR_EMAIL,
            relative_path,
            contents,
            message,
        )
    }

    /// Like [`TempRepo::commit_file`], but authored and committed by `name`
    /// `<email>` — for tests that assert on per-commit authorship, such as
    /// blame.
    pub fn commit_file_as(
        &self,
        name: &str,
        email: &str,
        relative_path: impl AsRef<Path>,
        contents: &str,
        message: &str,
    ) -> Oid {
        let relative_path = relative_path.as_ref();
        self.write_file(relative_path, contents);
        self.stage(relative_path);
        let tree_oid = self
            .repo
            .index()
            .expect("failed to open index")
            .write_tree()
            .expect("failed to write tree");
        let parents = self
            .repo
            .head()
            .ok()
            .and_then(|head| head.peel_to_commit().ok())
            .map(|parent| vec![parent.id()])
            .unwrap_or_default();
        let signature = self.signature_for(name, email);
        self.commit_tree_signed_by(Some("HEAD"), &signature, message, tree_oid, &parents)
    }

    /// Creates a branch called `name` at `HEAD`, leaving `HEAD` where it is.
    pub fn branch(&self, name: &str) {
        let head = self
            .repo
            .head()
            .expect("HEAD should exist")
            .peel_to_commit()
            .expect("HEAD should point at a commit");
        self.repo
            .branch(name, &head, false)
            .expect("failed to create branch");
    }

    fn signature_for(&self, name: &str, email: &str) -> Signature<'static> {
        let tick = self.signatures_issued.get();
        self.signatures_issued.set(tick + 1);
        let when = Time::new(FIRST_COMMIT_TIME + tick, 0);
        Signature::new(name, email, &when).expect("failed to create signature")
    }

    fn commit_tree_signed_by(
        &self,
        update_ref: Option<&str>,
        signature: &Signature,
        message: &str,
        tree_oid: Oid,
        parent_oids: &[Oid],
    ) -> Oid {
        let tree = self.repo.find_tree(tree_oid).expect("failed to find tree");
        let parents: Vec<Commit> = parent_oids
            .iter()
            .map(|oid| self.repo.find_commit(*oid).expect("failed to find parent"))
            .collect();
        let parent_refs: Vec<&Commit> = parents.iter().collect();

        self.repo
            .commit(
                update_ref,
                signature,
                signature,
                message,
                &tree,
                &parent_refs,
            )
            .expect("failed to commit")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commits_land_on_the_default_branch_whatever_git_is_configured_to_use() {
        let repo = TempRepo::init();
        let oid = repo.commit_file("a.txt", "a\n", "add a");

        let branch = repo
            .repo()
            .find_branch(TempRepo::DEFAULT_BRANCH, git2::BranchType::Local)
            .expect("default branch should exist");
        assert_eq!(branch.get().target(), Some(oid));
        assert_eq!(repo.head_oid(), oid);
    }

    #[test]
    fn commit_authorship_and_time_are_fixed() {
        let repo = TempRepo::with_commits(2);
        let tip = repo.repo().find_commit(repo.head_oid()).unwrap();

        assert_eq!(tip.author().name(), Some(TempRepo::AUTHOR_NAME));
        assert_eq!(tip.author().email(), Some(TempRepo::AUTHOR_EMAIL));
        // Second of two commits, so one second past the fixed start.
        assert_eq!(tip.time().seconds(), FIRST_COMMIT_TIME + 1);
        assert_eq!(tip.parent(0).unwrap().time().seconds(), FIRST_COMMIT_TIME);
    }

    #[test]
    fn a_committed_repo_is_identical_across_runs() {
        // The whole point of the fixed clock: the same script yields the same
        // OIDs, so a test may hard-code or compare them.
        let first = TempRepo::init();
        let second = TempRepo::init();
        assert_eq!(
            first.commit_file("a.txt", "a\n", "add a"),
            second.commit_file("a.txt", "a\n", "add a"),
        );
    }

    #[test]
    fn custom_tree_commits_use_successive_clock_ticks() {
        let repo = TempRepo::init();
        let tree_oid = repo.repo().index().unwrap().write_tree().unwrap();
        let first = repo.commit_tree(Some("refs/heads/side"), "first", tree_oid, &[]);
        let second = repo.commit_tree(Some("refs/heads/side"), "second", tree_oid, &[first]);

        let first = repo.repo().find_commit(first).unwrap();
        let second = repo.repo().find_commit(second).unwrap();
        assert_eq!(first.time().seconds(), FIRST_COMMIT_TIME);
        assert_eq!(second.time().seconds(), FIRST_COMMIT_TIME + 1);
    }

    #[test]
    fn requesting_a_signature_advances_the_fixture_clock() {
        let first = TempRepo::init();
        let reserved = first.signature();
        let commit = first.commit("after reserved signature");
        let commit = first.repo().find_commit(commit).unwrap();

        assert_eq!(reserved.when().seconds(), FIRST_COMMIT_TIME);
        assert_eq!(commit.time().seconds(), FIRST_COMMIT_TIME + 1);
    }

    #[test]
    fn write_file_leaves_the_change_unstaged() {
        let repo = TempRepo::init();
        repo.commit_file("a.txt", "a\n", "add a");
        repo.write_file("a.txt", "changed\n");

        let statuses = repo.repo().statuses(None).unwrap();
        assert!(statuses.iter().any(|entry| entry.status().is_wt_modified()));
    }

    #[test]
    fn nested_paths_get_their_parent_directories() {
        let repo = TempRepo::init();
        repo.commit_file("src/main.rs", "fn main() {}\n", "add main");
        assert!(repo.path().join("src/main.rs").exists());
    }

    #[test]
    fn the_temp_directory_outlives_the_binding_that_created_it() {
        let repo = TempRepo::with_commits(1);
        let path = repo.path().to_path_buf();
        assert!(path.exists());
        drop(repo);
        assert!(!path.exists(), "the directory should be cleaned up on drop");
    }
}
