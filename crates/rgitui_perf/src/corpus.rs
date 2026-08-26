//! Deterministic repositories to measure against.
//!
//! Benchmarks that run against whatever repositories happen to be on the
//! machine produce numbers nobody else can reproduce. The corpus is generated
//! from a fixed recipe with a fixed clock, so the same tier is byte-identical
//! on every machine and every run, and a regression is a change in the code
//! rather than a change in the input.
//!
//! Repositories are built with git2 plumbing — trees assembled directly and
//! committed without ever touching a working copy — because checking out
//! 20,000 commits would dominate generation time.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context as _;
use git2::build::CheckoutBuilder;
use git2::{
    Buf, Commit, FileMode, Mempack, Odb, Oid, Repository, RepositoryInitOptions, ResetType,
    Signature, StatusOptions, Time,
};
use serde::{Deserialize, Serialize};

/// A named corpus size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tier {
    /// 200 commits, one branch. For smoke-testing the harness itself.
    Tiny,
    /// 2,000 commits, 20 branches, ~300 files. A typical personal project.
    Small,
    /// 20,000 commits, 200 branches, ~5,000 files. A busy team repository,
    /// and the tier routine comparisons should use.
    Medium,
    /// Shapes chosen to break things rather than to be typical: wide merge
    /// fans, deep rename chains, a 50,000-line single-file diff, mixed CRLF
    /// and unicode paths.
    Pathological,
    /// 200,000 commits. Slow to generate and not needed for routine runs, so
    /// it is never built unless asked for by name.
    Large,
}

impl Tier {
    /// Every tier that [`ensure_all`] builds by default. [`Tier::Large`] is
    /// excluded deliberately — see its documentation.
    pub const DEFAULT: &'static [Tier] =
        &[Tier::Tiny, Tier::Small, Tier::Medium, Tier::Pathological];

    /// The tier's name as it appears on the command line and on disk.
    pub fn name(&self) -> &'static str {
        match self {
            Tier::Tiny => "tiny",
            Tier::Small => "small",
            Tier::Medium => "medium",
            Tier::Pathological => "pathological",
            Tier::Large => "large",
        }
    }

    /// Parses a tier name, listing the valid ones when it does not match.
    pub fn parse(name: &str) -> anyhow::Result<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "tiny" => Ok(Tier::Tiny),
            "small" => Ok(Tier::Small),
            "medium" => Ok(Tier::Medium),
            "pathological" => Ok(Tier::Pathological),
            "large" => Ok(Tier::Large),
            other => anyhow::bail!(
                "unknown corpus tier {other:?} — use tiny, small, medium, pathological or large"
            ),
        }
    }

    /// The generation parameters for this tier.
    pub fn recipe(&self) -> Recipe {
        match self {
            Tier::Tiny => Recipe {
                tier: *self,
                commits: 200,
                branches: 1,
                files: 20,
                max_file_lines: 200,
                merge_fan: 0,
                include_binary_blob: false,
            },
            Tier::Small => Recipe {
                tier: *self,
                commits: 2_000,
                branches: 20,
                files: 300,
                max_file_lines: 800,
                merge_fan: 2,
                include_binary_blob: false,
            },
            Tier::Medium => Recipe {
                tier: *self,
                commits: 20_000,
                branches: 200,
                files: 5_000,
                max_file_lines: 2_000,
                merge_fan: 4,
                include_binary_blob: true,
            },
            Tier::Pathological => Recipe {
                tier: *self,
                commits: 3_000,
                branches: 60,
                files: 400,
                max_file_lines: 50_000,
                merge_fan: 32,
                include_binary_blob: true,
            },
            Tier::Large => Recipe {
                tier: *self,
                commits: 200_000,
                branches: 1_000,
                files: 50_000,
                max_file_lines: 4_000,
                merge_fan: 8,
                include_binary_blob: true,
            },
        }
    }
}

/// The parameters that fully determine a generated repository.
///
/// Hashed into the on-disk directory name so that changing any parameter
/// regenerates rather than silently reusing a repository built to the old
/// recipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recipe {
    pub tier: Tier,
    /// Total commits across all branches.
    pub commits: usize,
    /// Branches created, including the default branch.
    pub branches: usize,
    /// Distinct files that ever exist in the tree.
    pub files: usize,
    /// Line count of the largest generated file.
    pub max_file_lines: usize,
    /// Parent count of the widest merge commit. Zero means no merges.
    pub merge_fan: usize,
    /// Whether to include an incompressible binary file, which exercises the
    /// diff viewer's binary-content path.
    pub include_binary_blob: bool,
}

/// Version of the generation algorithm.
///
/// Bumped whenever generation changes shape, so that existing corpora on disk
/// are treated as stale rather than compared against differently-built ones.
pub const GENERATOR_VERSION: u32 = 1;

/// Wall clock the first commit is stamped with; each later commit adds a
/// second. Fixed so commit OIDs never depend on when the corpus was built —
/// the same approach `rgitui_test_support::TempRepo` takes for tests.
pub const FIRST_COMMIT_TIME: i64 = 1_700_000_000;

impl Recipe {
    /// Stable fingerprint of every field plus [`GENERATOR_VERSION`].
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        GENERATOR_VERSION.hash(&mut hasher);
        self.tier.name().hash(&mut hasher);
        self.commits.hash(&mut hasher);
        self.branches.hash(&mut hasher);
        self.files.hash(&mut hasher);
        self.max_file_lines.hash(&mut hasher);
        self.merge_fan.hash(&mut hasher);
        self.include_binary_blob.hash(&mut hasher);
        hasher.finish()
    }

    /// Where a repository built to this recipe lives.
    pub fn path(&self) -> anyhow::Result<PathBuf> {
        let root = dirs::cache_dir()
            .ok_or_else(|| anyhow::anyhow!("no user cache directory to hold the perf corpus"))?
            .join("rgitui")
            .join("perf-corpus");
        Ok(root.join(format!("{}-{:016x}", self.tier.name(), self.fingerprint())))
    }
}

/// Returns the path to a generated repository for `tier`, building it if the
/// recipe has changed or it does not exist yet.
pub fn ensure(tier: Tier) -> anyhow::Result<PathBuf> {
    let recipe = tier.recipe();
    let path = recipe.path()?;
    ensure_at(&recipe, &path)
}

/// Builds every tier in [`Tier::DEFAULT`].
pub fn ensure_all() -> anyhow::Result<Vec<PathBuf>> {
    Tier::DEFAULT.iter().copied().map(ensure).collect()
}

/// Identity on every commit. Fixed for the same reason the clock is: the
/// machine's git config must not be able to change an OID.
const AUTHOR_NAME: &str = "rgitui perf";
/// Email that goes with [`AUTHOR_NAME`], on a domain that can never resolve.
const AUTHOR_EMAIL: &str = "perf@rgitui.invalid";
/// Branch the trunk of the generated history lives on.
const DEFAULT_BRANCH: &str = "main";

/// Marks a corpus as finished, and records the recipe it was built to.
///
/// Written after every object, ref and checked-out file is in place, so that a
/// generation killed halfway leaves something visibly incomplete rather than
/// something that looks whole and is quietly short a few thousand commits. It
/// lives inside `.git` so the working tree a benchmark opens has no untracked
/// file of ours sitting in its status.
const MARKER_FILE: &str = ".rgitui-corpus-complete";

/// Path of the file added when [`Recipe::include_binary_blob`] is set.
const BINARY_FILE: [&str; 2] = ["assets", "texture.bin"];

/// Size of that file. Big enough that reading it is worth measuring, small
/// enough that its handful of revisions do not dominate the corpus on disk.
const BINARY_BYTES: usize = 256 * 1024;

/// Longest an ordinary generated file gets, whatever the recipe's ceiling.
/// Only the deliberately large files go past this.
const TYPICAL_FILE_LINES: usize = 400;

/// Priority of the in-memory object backend, above libgit2's loose (1) and
/// pack (2) backends so that every object written during generation lands in
/// memory instead of in a file of its own.
const MEMPACK_PRIORITY: i32 = 1_000;

/// Commits between packfile flushes, which bounds how much of the object
/// database is held in memory at once.
const COMMITS_PER_PACK: usize = 8_192;

/// Renames per chain, and number of chains, for [`Tier::Pathological`].
/// Several medium chains give rename detection more trails to follow than one
/// long one would.
const RENAME_CHAIN_LENGTH: usize = 12;
const RENAME_CHAINS: usize = 3;

/// [`ensure`] with the destination named explicitly, which is what lets tests
/// generate into a temporary directory instead of the user's cache.
///
/// Generation goes into a sibling directory and is renamed into place once it
/// is finished, so the destination is only ever absent or complete.
fn ensure_at(recipe: &Recipe, path: &Path) -> anyhow::Result<PathBuf> {
    let tier = recipe.tier.name();
    if is_complete(recipe, path) {
        log::info!("perf corpus {tier} is already built at {}", path.display());
        restore(path)?;
        return Ok(path.to_path_buf());
    }

    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("corpus path {} has no parent directory", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("corpus path {} has no directory name", path.display()))?
        .to_string_lossy()
        .into_owned();
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;

    // Anything already at either path was left by an older generator or by a
    // run that died partway through, and neither can be told from a whole
    // corpus by looking at it.
    clear(path)?;
    let staging = parent.join(format!(".{name}.partial"));
    clear(&staging)?;

    let Recipe {
        commits,
        branches,
        files,
        ..
    } = *recipe;
    log::info!(
        "generating the {tier} perf corpus — {commits} commits, {branches} branches, {files} files — into {}",
        path.display()
    );
    let started = Instant::now();
    generate(recipe, &staging)?;
    std::fs::rename(&staging, path).with_context(|| {
        format!(
            "failed to move the generated corpus from {} to {}",
            staging.display(),
            path.display()
        )
    })?;

    let seconds = started.elapsed().as_secs_f64();
    log::info!("generated the {tier} perf corpus in {seconds:.1}s");
    Ok(path.to_path_buf())
}

/// Put a cached corpus back the way the generator left it.
///
/// The completion marker records the recipe, which says nothing about the
/// working tree, and a scenario is free to change it — `staging-churn` dirties
/// forty files on purpose. Left alone that compounds: every run appends to the
/// files the last one appended to, so file sizes and diff sizes grow run over
/// run and two runs of the same scenario stop measuring the same thing, which
/// is the one guarantee a benchmark corpus has to make. Restoring when the
/// corpus is handed out, rather than when a run ends, also covers the run that
/// panicked halfway through and never got to tidy up.
fn restore(root: &Path) -> anyhow::Result<()> {
    let repo = Repository::open(root)
        .with_context(|| format!("failed to open the corpus at {}", root.display()))?;
    if repo.workdir().is_none() {
        return Ok(());
    }

    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);
    let statuses = repo.statuses(Some(&mut options))?;
    if statuses.is_empty() {
        return Ok(());
    }
    let dirty = statuses.len();

    // A hard reset restores tracked content and clears the index, but leaves
    // whatever the run created behind: `GIT_CHECKOUT_REMOVE_UNTRACKED` only
    // reaches paths the checkout itself walks, and resetting to the commit
    // already at HEAD walks almost nothing. Untracked paths are removed here,
    // from the status list, so the guarantee does not depend on that.
    let workdir = repo.workdir().unwrap_or(root).to_path_buf();
    for entry in statuses.iter() {
        if !entry.status().contains(git2::Status::WT_NEW) {
            continue;
        }
        let Some(path) = entry.path() else { continue };
        let full = workdir.join(path);
        if let Err(error) = std::fs::remove_file(&full) {
            log::warn!("could not remove {}: {error}", full.display());
        }
    }
    drop(statuses);

    let head = repo.head()?.peel(git2::ObjectType::Commit)?;
    let mut checkout = CheckoutBuilder::new();
    checkout.force();
    repo.reset(&head, ResetType::Hard, Some(&mut checkout))
        .with_context(|| format!("failed to restore the corpus at {}", root.display()))?;
    log::info!(
        "restored {dirty} path(s) a previous run left changed in {}",
        root.display()
    );
    Ok(())
}

/// Where the completion marker for a corpus rooted at `root` lives.
fn marker_path(root: &Path) -> PathBuf {
    root.join(".git").join(MARKER_FILE)
}

/// Whether `root` holds a corpus that finished building to exactly `recipe`.
fn is_complete(recipe: &Recipe, root: &Path) -> bool {
    let Ok(marker) = std::fs::read_to_string(marker_path(root)) else {
        return false;
    };
    serde_json::from_str::<Recipe>(&marker).is_ok_and(|built| built == *recipe)
}

/// Removes `path` and everything under it, treating "it was not there" as
/// success.
fn clear(path: &Path) -> anyhow::Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(anyhow::Error::new(error).context(format!("failed to clear {}", path.display())))
        }
    }
}

/// Builds a repository at `root` and marks it complete.
fn generate(recipe: &Recipe, root: &Path) -> anyhow::Result<()> {
    let mut options = RepositoryInitOptions::new();
    options.initial_head(DEFAULT_BRANCH);
    let repo = Repository::init_opts(root, &options)
        .with_context(|| format!("failed to create a repository at {}", root.display()))?;

    {
        let mut config = repo.config()?;
        config.set_str("user.name", AUTHOR_NAME)?;
        config.set_str("user.email", AUTHOR_EMAIL)?;
        // The pathological tier commits CRLF files on purpose, and a machine
        // with core.autocrlf set globally would rewrite them on checkout.
        config.set_bool("core.autocrlf", false)?;
    }

    // Scoped so the object database and its in-memory backend are closed
    // before the repository is.
    {
        let odb = repo.odb()?;
        let mempack = odb.add_new_mempack_backend(MEMPACK_PRIORITY)?;
        Generator::new(&repo, &odb, &mempack, *recipe).run()?;
    }

    // Windows refuses to rename a directory that still has open handles under
    // it, and libgit2 holds the object database and the index open until the
    // repository drops.
    drop(repo);

    std::fs::write(marker_path(root), serde_json::to_string_pretty(recipe)?)
        .with_context(|| format!("failed to mark {} complete", root.display()))?;
    Ok(())
}

/// SplitMix64, written out here rather than taken as a dependency.
///
/// The corpus needs a generator whose output is fixed forever, and a crate is
/// free to change its algorithm in a point release — which would silently
/// rewrite every OID in every corpus.
struct Prng(u64);

impl Prng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// A value in `0..limit`, and zero when `limit` is zero.
    fn below(&mut self, limit: usize) -> usize {
        if limit == 0 {
            return 0;
        }
        (self.next_u64() % limit as u64) as usize
    }
}

/// What one commit does to a set of paths: a new blob, or `None` to delete.
///
/// Paths are kept split into components because that is the shape trees nest
/// in, and the map is ordered so the same recipe always walks them in the same
/// order.
type Changes = BTreeMap<Vec<String>, Option<Oid>>;

/// One entry of [`Changes`] with its path borrowed, for the tree walk.
type Change<'a> = (&'a [String], Option<Oid>);

/// A generated text file, held as one seed per line.
///
/// Seeds rather than text keep a 50,000-line file down to 200KB of state and
/// make an edit a handful of `u32` writes; the text itself only exists for as
/// long as it takes to hand libgit2 a blob.
struct TextFile {
    path: Vec<String>,
    lines: Vec<u32>,
    /// `\r\n` for the files the pathological tier commits with CRLF endings.
    newline: &'static str,
    alive: bool,
}

impl TextFile {
    fn render(&self) -> String {
        let mut text = String::with_capacity(self.lines.len() * 48);
        for seed in &self.lines {
            append_line(&mut text, *seed);
            text.push_str(self.newline);
        }
        text
    }

    fn name(&self) -> &str {
        self.path.last().map_or("", String::as_str)
    }
}

/// A side branch under construction.
struct Lane {
    name: String,
    /// Tip commit, which is where the branch ref ends up pointing.
    tip: Oid,
    tree: Oid,
    /// Every path this lane has touched since it forked. Replaying these onto
    /// the trunk is what makes a merge keep both sides' work instead of
    /// resetting the trunk to the fork point.
    changes: Changes,
    /// Commits still owed before the lane is ready to merge.
    remaining: usize,
}

/// Vocabulary the generated content is assembled from — fixed arrays, so the
/// same seed always picks the same word.
const IDENTS: [&str; 16] = [
    "buffer", "cursor", "handle", "index", "record", "session", "window", "layout", "cache",
    "parser", "matcher", "walker", "budget", "sample", "channel", "registry",
];
const TYPES: [&str; 8] = [
    "usize",
    "String",
    "Oid",
    "Range<u32>",
    "Arc<Repo>",
    "bool",
    "PathBuf",
    "Duration",
];
const VERBS: [&str; 8] = [
    "Refine", "Tidy", "Extend", "Simplify", "Harden", "Rework", "Trim", "Adjust",
];
const BODIES: [&str; 4] = [
    "The old path did the work twice and threw half of it away. Do it once\nand keep the result.",
    "Splitting this out means the caller no longer has to know which of the\ntwo halves runs first.",
    "Measured before and after: the same output, one allocation per row\ninstead of three.",
    "No behaviour change. The names described the shape this used to have\nrather than the one it has.",
];
/// Top-level directories, so paths look like a project rather than a flat bag.
const TOP_DIRS: [&str; 6] = ["src", "crates", "lib", "tests", "docs", "tools"];
const EXTENSIONS: [&str; 7] = ["rs", "ts", "py", "go", "md", "json", "toml"];
/// Non-ASCII path components for the pathological tier, so that nothing along
/// the path-handling chain gets away with assuming ASCII.
const UNICODE_DIRS: [&str; 4] = ["文档", "ünïcödé", "документы", "größe"];
const UNICODE_STEMS: [&str; 4] = ["说明", "résumé", "файл", "straße"];

/// Writes one line of plausible source text for `seed`.
fn append_line(text: &mut String, seed: u32) {
    let seed = seed as usize;
    let first = IDENTS[seed % IDENTS.len()];
    let second = IDENTS[(seed >> 4) % IDENTS.len()];
    let ty = TYPES[(seed >> 8) % TYPES.len()];
    let number = (seed >> 12) % 4096;
    let written = match seed % 6 {
        0 => write!(
            text,
            "    let {first}_{number} = {second}.len() + {number};"
        ),
        1 => write!(
            text,
            "    if {first}.is_empty() {{ return Err(Error::Missing{number}); }}"
        ),
        2 => write!(text, "    // {first} feeds {second} on pass {number}"),
        3 => write!(
            text,
            "    pub fn {first}_{number}(&self, {second}: {ty}) -> {ty} {{ self.{first} }}"
        ),
        4 => write!(text, "    {first}.push({second}_{number});"),
        _ => write!(
            text,
            "    debug_assert!({first}_{number} <= {second}.capacity());"
        ),
    };
    written.expect("writing into a String cannot fail");
}

/// Where the rename chain's file sits after `step` renames.
fn rename_target(step: usize) -> Vec<String> {
    let chain = step / RENAME_CHAIN_LENGTH;
    let depth = step % RENAME_CHAIN_LENGTH;
    let mut path = vec!["legacy".to_string(), format!("chain_{chain}")];
    // Each rename also buries the file a directory deeper, so a chain moves
    // across the tree instead of shuffling names inside one directory.
    path.extend((0..=depth).map(|level| format!("v{level}")));
    path.push(format!("moved_{step:02}.rs"));
    path
}

/// Applies `changes` to the tree `base` and returns the tree that results.
fn apply_changes(repo: &Repository, base: Option<Oid>, changes: &Changes) -> anyhow::Result<Oid> {
    let flattened: Vec<Change<'_>> = changes
        .iter()
        .map(|(path, blob)| (path.as_slice(), *blob))
        .collect();
    Ok(build_tree(repo, base, &flattened)?.0)
}

/// Rebuilds one level of a tree, recursing only into the directories that
/// `changes` touches.
///
/// Everything else is inherited by seeding the builder from `base`, which is
/// what keeps a commit's cost proportional to what it changed rather than to
/// the size of the tree. `changes` must be sorted by path components so that
/// entries sharing a directory arrive together.
///
/// Returns the new tree and its entry count; the caller needs the count to
/// drop a directory a deletion has just emptied rather than record an empty
/// tree in its parent.
fn build_tree(
    repo: &Repository,
    base: Option<Oid>,
    changes: &[Change<'_>],
) -> anyhow::Result<(Oid, usize)> {
    let base = base.map(|oid| repo.find_tree(oid)).transpose()?;
    let mut builder = repo.treebuilder(base.as_ref())?;

    let mut start = 0;
    while start < changes.len() {
        let name = changes[start].0[0].as_str();
        let mut end = start + 1;
        while end < changes.len() && changes[end].0[0] == name {
            end += 1;
        }

        if changes[start].0.len() == 1 {
            match changes[start].1 {
                Some(blob) => {
                    builder.insert(name, blob, i32::from(FileMode::Blob))?;
                }
                None => {
                    if builder.get(name)?.is_some() {
                        builder.remove(name)?;
                    }
                }
            }
        } else {
            let nested: Vec<Change<'_>> = changes[start..end]
                .iter()
                .map(|(path, blob)| (&path[1..], *blob))
                .collect();
            let nested_base = builder.get(name)?.map(|entry| entry.id());
            let (tree, entries) = build_tree(repo, nested_base, &nested)?;
            if entries == 0 {
                if builder.get(name)?.is_some() {
                    builder.remove(name)?;
                }
            } else {
                builder.insert(name, tree, i32::from(FileMode::Tree))?;
            }
        }

        start = end;
    }

    let entries = builder.len();
    Ok((builder.write()?, entries))
}

/// Builds one repository to one recipe.
struct Generator<'repo> {
    repo: &'repo Repository,
    odb: &'repo Odb<'repo>,
    /// Where every object is written before it is packed. See
    /// [`Generator::flush_objects`].
    mempack: &'repo Mempack<'repo>,
    recipe: Recipe,
    rng: Prng,
    /// Every file that has ever existed, in creation order.
    files: Vec<TextFile>,
    /// Files present in the root commit; the rest appear across the history.
    seeded: usize,
    /// Commits written so far, which is also the tick of the fixed clock.
    commits: usize,
    trunk_commits: usize,
    binary_revisions: usize,
    rename_steps: usize,
    progress_every: usize,
    /// Commit count at the last packfile flush.
    packed_at: usize,
}

impl<'repo> Generator<'repo> {
    fn new(
        repo: &'repo Repository,
        odb: &'repo Odb<'repo>,
        mempack: &'repo Mempack<'repo>,
        recipe: Recipe,
    ) -> Self {
        Self {
            repo,
            odb,
            mempack,
            recipe,
            // Seeded from the recipe so no two tiers share a stream and the
            // same tier always replays the same one.
            rng: Prng::new(recipe.fingerprint()),
            files: Vec::with_capacity(recipe.files),
            seeded: if recipe.files == 0 {
                0
            } else {
                (recipe.files / 10).max(1)
            },
            commits: 0,
            trunk_commits: 0,
            binary_revisions: 0,
            rename_steps: 0,
            progress_every: (recipe.commits / 8).clamp(200, 2_000),
            packed_at: 0,
        }
    }

    /// Moves everything written since the last flush out of memory and into a
    /// packfile.
    ///
    /// Objects are written to an in-memory backend and packed in batches
    /// rather than dropped on disk one file at a time. A medium corpus is well
    /// over a hundred thousand objects, and creating that many files costs
    /// minutes on a filesystem where creating one costs a millisecond. It also
    /// leaves the corpus in the shape real repositories are in — packed —
    /// rather than one no user's repository is ever in.
    fn flush_objects(&mut self) -> anyhow::Result<()> {
        if self.commits == self.packed_at {
            return Ok(());
        }
        let mut pack = Buf::new();
        self.mempack.dump(self.repo, &mut pack)?;

        let mut writer = self.odb.packwriter()?;
        writer.write_all(&pack)?;
        writer.commit()?;

        // Only once the pack is on disk can the in-memory copies go; until
        // then they are the only copy the tree and commit lookups can read.
        self.mempack.reset()?;
        self.packed_at = self.commits;
        Ok(())
    }

    fn run(&mut self) -> anyhow::Result<()> {
        let mut changes = Changes::new();
        while self.files.len() < self.seeded {
            self.create_file(&mut changes)?;
        }
        if self.recipe.include_binary_blob {
            self.revise_binary(&mut changes)?;
        }
        let mut trunk_tree = apply_changes(self.repo, None, &changes)?;
        let mut trunk_tip = self.commit(trunk_tree, &[], "Initial import")?;

        // Lanes are open in parallel so that several branches are live at once
        // and the graph's lanes actually cross, and so that there are always
        // at least as many as the widest merge needs parents.
        let lane_capacity = if self.recipe.merge_fan == 0 {
            0
        } else {
            self.recipe
                .merge_fan
                .max(4)
                .min(self.recipe.branches.saturating_sub(1))
        };
        let mut lanes: Vec<Lane> = Vec::new();
        let mut refs: Vec<(String, Oid)> = Vec::new();
        let mut branches = 1;
        let mut merges = 0;

        while self.commits < self.recipe.commits {
            if lanes.len() < lane_capacity && branches < self.recipe.branches {
                let lane = self.open_lane(trunk_tip, trunk_tree, branches);
                lanes.push(lane);
                branches += 1;
                continue;
            }

            let ready = lanes.iter().filter(|lane| lane.remaining == 0).count();
            // Most merges take one branch; every sixth takes the recipe's full
            // fan, which is where the octopus merges come from.
            let wide = self.recipe.merge_fan >= 3 && merges % 6 == 5;
            let wanted = if wide { self.recipe.merge_fan - 1 } else { 1 };
            // Settling for fewer than `wanted` once every lane is ready is
            // what stops a fan wider than the pool from waiting forever.
            if ready > 0 && (ready >= wanted || ready == lanes.len()) {
                let mut merging = Vec::new();
                let mut index = 0;
                while index < lanes.len() && merging.len() < wanted {
                    if lanes[index].remaining == 0 {
                        merging.push(lanes.remove(index));
                    } else {
                        index += 1;
                    }
                }
                let (tip, tree) = self.merge(trunk_tip, trunk_tree, &merging)?;
                trunk_tip = tip;
                trunk_tree = tree;
                refs.extend(merging.into_iter().map(|lane| (lane.name, lane.tip)));
                merges += 1;
                continue;
            }

            // Most work in a real repository lands on a branch, so the trunk
            // takes the minority of direct commits.
            let on_lane = if lanes.is_empty() || self.rng.below(4) == 0 {
                None
            } else {
                self.pick_lane(&lanes)
            };
            match on_lane {
                Some(index) => {
                    let (changes, message) = self.edit(false)?;
                    let tree = apply_changes(self.repo, Some(lanes[index].tree), &changes)?;
                    let tip = self.commit(tree, &[lanes[index].tip], &message)?;
                    let lane = &mut lanes[index];
                    lane.tree = tree;
                    lane.tip = tip;
                    lane.remaining -= 1;
                    lane.changes.extend(changes);
                }
                None => {
                    let (changes, message) = self.edit(true)?;
                    trunk_tree = apply_changes(self.repo, Some(trunk_tree), &changes)?;
                    trunk_tip = self.commit(trunk_tree, &[trunk_tip], &message)?;
                }
            }
        }

        // Everything still in memory has to reach disk before the checkout
        // reads it back and before the repository closes.
        self.flush_objects()?;

        refs.extend(lanes.into_iter().map(|lane| (lane.name, lane.tip)));
        // The recipe promises a branch count. If the commit budget ran out
        // before every lane could be opened, the rest still get their refs.
        while branches < self.recipe.branches {
            refs.push((self.lane_name(branches), trunk_tip));
            branches += 1;
        }

        // Refs are written once, here, rather than on every commit: a ref
        // update and its reflog entry on each of 20,000 commits is pure
        // overhead for a value nothing reads until generation is over.
        self.repo.reference(
            &format!("refs/heads/{DEFAULT_BRANCH}"),
            trunk_tip,
            true,
            "corpus generation",
        )?;
        for (name, oid) in &refs {
            self.repo.reference(
                &format!("refs/heads/{name}"),
                *oid,
                true,
                "corpus generation",
            )?;
        }

        // One checkout, at the end, rather than one per commit: without it
        // every benchmark that asked for status would be measuring a working
        // tree that reports the entire repository as deleted.
        let mut checkout = CheckoutBuilder::new();
        checkout.force();
        self.repo.checkout_head(Some(&mut checkout))?;

        let commits = self.commits;
        let branch_count = refs.len() + 1;
        let files = self.files.len();
        log::info!("wrote {commits} commits, {branch_count} branches and {files} files");
        Ok(())
    }

    /// Picks the changes and the message for one ordinary commit.
    fn edit(&mut self, on_trunk: bool) -> anyhow::Result<(Changes, String)> {
        let mut changes = Changes::new();

        // Files arrive across the history rather than all in the root commit,
        // so that additions are part of what the benchmarks walk.
        let span = self.recipe.commits.saturating_sub(1).max(1);
        let growth = self.recipe.files.saturating_sub(self.seeded);
        let target = (self.seeded + growth * self.commits / span).min(self.recipe.files);
        while self.files.len() < target {
            self.create_file(&mut changes)?;
        }

        let mut headline = String::new();
        for _ in 0..1 + self.rng.below(3) {
            let Some(index) = self.pick_live_file() else {
                break;
            };
            if changes.contains_key(&self.files[index].path) {
                continue;
            }
            self.modify_file(index, &mut changes)?;
            if headline.is_empty() {
                headline = self.files[index].name().to_string();
            }
        }

        if on_trunk {
            if self.recipe.include_binary_blob
                && self.binary_revisions <= 3
                && self.trunk_commits % 97 == 96
            {
                self.revise_binary(&mut changes)?;
            }
            if self.recipe.tier == Tier::Pathological
                && self.rename_steps < RENAME_CHAIN_LENGTH * RENAME_CHAINS
                && self.trunk_commits % 3 == 2
            {
                self.rename(&mut changes)?;
            }
            if self.trunk_commits % 51 == 50 {
                self.delete_file(&mut changes);
            }
            self.trunk_commits += 1;
        }

        let verb = VERBS[self.rng.below(VERBS.len())];
        let mut message = if headline.is_empty() {
            format!("{verb} the layout")
        } else {
            format!("{verb} {headline}")
        };
        // A quarter of commits carry a body, because the detail panel renders
        // one and a corpus of subject-only commits would never exercise it.
        if self.rng.below(4) == 0 {
            let body = BODIES[self.rng.below(BODIES.len())];
            message.push_str("\n\n");
            message.push_str(body);
            message.push('\n');
        }
        Ok((changes, message))
    }

    /// Adds the next file in the recipe's sequence.
    fn create_file(&mut self, changes: &mut Changes) -> anyhow::Result<()> {
        let index = self.files.len();
        let path = self.file_path(index);
        let lines = self.file_lines(index);
        let mut seeds = Vec::with_capacity(lines);
        for _ in 0..lines {
            seeds.push(self.rng.next_u32());
        }
        let file = TextFile {
            path,
            lines: seeds,
            // A minority of the pathological tier's files are committed with
            // CRLF endings, mixed in among LF ones.
            newline: if self.recipe.tier == Tier::Pathological && index % 7 == 3 {
                "\r\n"
            } else {
                "\n"
            },
            alive: true,
        };
        let blob = self.repo.blob(file.render().as_bytes())?;
        changes.insert(file.path.clone(), Some(blob));
        self.files.push(file);
        Ok(())
    }

    /// Rewrites a few of a file's lines and returns the new blob.
    ///
    /// A few lines rather than the whole file, so that diffs come out as hunks
    /// with context around them — the case the diff viewer spends its time on
    /// — instead of one whole-file replacement.
    fn modify_file(&mut self, index: usize, changes: &mut Changes) -> anyhow::Result<Oid> {
        let Self {
            repo,
            files,
            rng,
            recipe,
            ..
        } = self;
        let file = &mut files[index];

        for _ in 0..1 + rng.below(5) {
            if file.lines.is_empty() {
                break;
            }
            let line = rng.below(file.lines.len());
            file.lines[line] = rng.next_u32();
        }

        // Files grow and shrink as well as change in place; edits that only
        // ever replace lines never produce a pure insertion or deletion hunk.
        // Growth stops at the recipe's ceiling so that the field keeps meaning
        // what it says.
        match rng.below(4) {
            0 if file.lines.len() < recipe.max_file_lines => {
                let room = recipe.max_file_lines - file.lines.len();
                let at = rng.below(file.lines.len() + 1);
                for offset in 0..(1 + rng.below(6)).min(room) {
                    let seed = rng.next_u32();
                    file.lines.insert(at + offset, seed);
                }
            }
            1 if file.lines.len() > 8 => {
                let at = rng.below(file.lines.len() - 4);
                let end = (at + 1 + rng.below(3)).min(file.lines.len());
                file.lines.drain(at..end);
            }
            _ => {}
        }

        let blob = repo.blob(file.render().as_bytes())?;
        changes.insert(file.path.clone(), Some(blob));
        Ok(blob)
    }

    /// Moves the current rename chain's file one step along.
    fn rename(&mut self, changes: &mut Changes) -> anyhow::Result<()> {
        let step = self.rename_steps;
        let index = 2 + step / RENAME_CHAIN_LENGTH;
        if index >= self.files.len() {
            return Ok(());
        }
        self.rename_steps += 1;

        // The content moves as well as the path, so rename detection has to
        // work on similar rather than identical blobs.
        let blob = self.modify_file(index, changes)?;
        let previous = std::mem::replace(&mut self.files[index].path, rename_target(step));
        changes.insert(previous, None);
        changes.insert(self.files[index].path.clone(), Some(blob));
        Ok(())
    }

    /// Drops a file from the tree, so that history contains deletions too.
    fn delete_file(&mut self, changes: &mut Changes) {
        let Some(index) = self.pick_live_file() else {
            return;
        };
        // The first few files are the README, the recipe's largest file, and
        // the ones the rename chains walk; those have to stay.
        if index < 8 {
            return;
        }
        self.files[index].alive = false;
        changes.insert(self.files[index].path.clone(), None);
    }

    /// Writes a new revision of the incompressible binary file.
    fn revise_binary(&mut self, changes: &mut Changes) -> anyhow::Result<()> {
        let mut data = vec![0u8; BINARY_BYTES];
        for chunk in data.chunks_mut(8) {
            let word = self.rng.next_u64().to_le_bytes();
            chunk.copy_from_slice(&word[..chunk.len()]);
        }
        let blob = self.repo.blob(&data)?;
        changes.insert(
            BINARY_FILE.iter().map(|part| (*part).to_string()).collect(),
            Some(blob),
        );
        self.binary_revisions += 1;
        Ok(())
    }

    /// Merges `lanes` into the trunk, returning the new tip and tree.
    fn merge(
        &mut self,
        trunk_tip: Oid,
        trunk_tree: Oid,
        lanes: &[Lane],
    ) -> anyhow::Result<(Oid, Oid)> {
        let mut changes = Changes::new();
        for lane in lanes {
            for (path, blob) in &lane.changes {
                changes.insert(path.clone(), *blob);
            }
        }
        let tree = apply_changes(self.repo, Some(trunk_tree), &changes)?;

        let mut parents = Vec::with_capacity(lanes.len() + 1);
        parents.push(trunk_tip);
        parents.extend(lanes.iter().map(|lane| lane.tip));

        let message = match lanes {
            [lane] => format!("Merge branch '{}' into {DEFAULT_BRANCH}", lane.name),
            many => format!("Merge {} branches into {DEFAULT_BRANCH}", many.len()),
        };
        let tip = self.commit(tree, &parents, &message)?;
        Ok((tip, tree))
    }

    fn open_lane(&mut self, tip: Oid, tree: Oid, ordinal: usize) -> Lane {
        Lane {
            name: self.lane_name(ordinal),
            tip,
            tree,
            changes: Changes::new(),
            remaining: 1 + self.rng.below(5),
        }
    }

    fn lane_name(&mut self, ordinal: usize) -> String {
        const PREFIXES: [&str; 3] = ["feature", "bugfix", "release"];
        let prefix = PREFIXES[ordinal % PREFIXES.len()];
        let topic = IDENTS[self.rng.below(IDENTS.len())];
        format!("{prefix}/{ordinal:04}-{topic}")
    }

    /// Picks a lane that still owes commits.
    fn pick_lane(&mut self, lanes: &[Lane]) -> Option<usize> {
        let candidates: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| lane.remaining > 0)
            .map(|(index, _)| index)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        Some(candidates[self.rng.below(candidates.len())])
    }

    /// Picks a file that has not been deleted, starting from a random one.
    fn pick_live_file(&mut self) -> Option<usize> {
        if self.files.is_empty() {
            return None;
        }
        let start = self.rng.below(self.files.len());
        (0..self.files.len())
            .map(|offset| (start + offset) % self.files.len())
            .find(|index| self.files[*index].alive)
    }

    fn file_path(&self, index: usize) -> Vec<String> {
        if index == 0 {
            return vec!["README.md".to_string()];
        }
        let extension = EXTENSIONS[index % EXTENSIONS.len()];
        if self.recipe.tier == Tier::Pathological && index % 11 == 5 {
            let directory = UNICODE_DIRS[index % UNICODE_DIRS.len()];
            let stem = UNICODE_STEMS[(index / 3) % UNICODE_STEMS.len()];
            return vec![
                directory.to_string(),
                format!("{stem}_{index:05}.{extension}"),
            ];
        }
        let group = index / 24;
        vec![
            TOP_DIRS[group % TOP_DIRS.len()].to_string(),
            format!("module_{group:03}"),
            format!("file_{index:05}.{extension}"),
        ]
    }

    /// How many lines a newly created file starts with.
    ///
    /// The everyday size does not scale with the recipe's ceiling: a tier that
    /// exists to hold one 50,000-line file should not also hold four hundred
    /// files averaging six thousand lines, which would cost gigabytes on disk
    /// and measure nothing the big file does not.
    fn file_lines(&mut self, index: usize) -> usize {
        let max = self.recipe.max_file_lines.max(1);
        match index {
            0 => max.min(12),
            // The recipe names the largest file's length, so exactly one file
            // is that long and every other one sits under it.
            1 => max,
            _ if index.is_multiple_of(64) => max / 2 + self.rng.below(max / 2 + 1),
            _ => 20 + self.rng.below(max.min(TYPICAL_FILE_LINES) + 1),
        }
    }

    fn commit(&mut self, tree: Oid, parents: &[Oid], message: &str) -> anyhow::Result<Oid> {
        let when = Time::new(FIRST_COMMIT_TIME + self.commits as i64, 0);
        let signature = Signature::new(AUTHOR_NAME, AUTHOR_EMAIL, &when)?;
        let tree = self.repo.find_tree(tree)?;
        let parents = parents
            .iter()
            .map(|oid| self.repo.find_commit(*oid))
            .collect::<Result<Vec<Commit<'_>>, _>>()?;
        let parents: Vec<&Commit<'_>> = parents.iter().collect();

        let oid = self
            .repo
            .commit(None, &signature, &signature, message, &tree, &parents)?;
        self.commits += 1;
        if self.commits.is_multiple_of(self.progress_every) {
            let done = self.commits;
            let total = self.recipe.commits;
            log::info!("  {done} of {total} commits");
        }
        if self.commits.is_multiple_of(COMMITS_PER_PACK) {
            self.flush_objects()?;
        }
        Ok(oid)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use git2::{BranchType, ObjectType, TreeWalkMode, TreeWalkResult};
    use tempfile::TempDir;

    use super::*;

    /// A recipe small enough to generate inside a test, with every shape the
    /// generator can produce still switched on.
    fn tiny_recipe() -> Recipe {
        Recipe {
            tier: Tier::Tiny,
            commits: 30,
            branches: 4,
            files: 12,
            max_file_lines: 40,
            merge_fan: 2,
            include_binary_blob: true,
        }
    }

    /// Small, but with enough branches to reach the sixth merge — which is the
    /// first one to open the recipe's full fan.
    fn pathological_recipe() -> Recipe {
        Recipe {
            tier: Tier::Pathological,
            commits: 120,
            branches: 16,
            files: 40,
            max_file_lines: 50,
            merge_fan: 3,
            include_binary_blob: false,
        }
    }

    /// Every path in every local branch's tree.
    fn all_paths(repo: &Repository) -> BTreeSet<String> {
        let mut paths = BTreeSet::new();
        for branch in repo.branches(Some(BranchType::Local)).unwrap() {
            let (branch, _) = branch.unwrap();
            let tree = branch.get().peel_to_commit().unwrap().tree().unwrap();
            tree.walk(TreeWalkMode::PreOrder, |root, entry| {
                if entry.kind() == Some(ObjectType::Blob) {
                    paths.insert(format!("{root}{}", entry.name().unwrap_or_default()));
                }
                TreeWalkResult::Ok
            })
            .unwrap();
        }
        paths
    }

    /// Every blob that any commit in the history ever contained.
    fn all_blobs(repo: &Repository) -> BTreeSet<Oid> {
        let mut walk = repo.revwalk().unwrap();
        walk.push_glob("refs/heads/*").unwrap();
        let mut blobs = BTreeSet::new();
        for oid in walk {
            let commit = repo.find_commit(oid.unwrap()).unwrap();
            commit
                .tree()
                .unwrap()
                .walk(TreeWalkMode::PreOrder, |_, entry| {
                    if entry.kind() == Some(ObjectType::Blob) {
                        blobs.insert(entry.id());
                    }
                    TreeWalkResult::Ok
                })
                .unwrap();
        }
        blobs
    }

    fn head_oid(path: &Path) -> Oid {
        let repo = Repository::open(path).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        head.id()
    }

    fn refs_of(path: &Path) -> Vec<(String, Oid)> {
        let repo = Repository::open(path).unwrap();
        let mut refs: Vec<(String, Oid)> = repo
            .branches(Some(BranchType::Local))
            .unwrap()
            .map(|branch| {
                let (branch, _) = branch.unwrap();
                let name = branch.name().unwrap().unwrap().to_string();
                (name, branch.get().target().unwrap())
            })
            .collect();
        refs.sort();
        refs
    }

    #[test]
    fn tier_names_round_trip() {
        for tier in [
            Tier::Tiny,
            Tier::Small,
            Tier::Medium,
            Tier::Pathological,
            Tier::Large,
        ] {
            assert_eq!(Tier::parse(tier.name()).unwrap(), tier);
            assert_eq!(Tier::parse(&tier.name().to_uppercase()).unwrap(), tier);
            assert_eq!(tier.recipe().tier, tier);
        }
    }

    #[test]
    fn parse_rejects_an_unknown_tier_by_listing_the_real_ones() {
        let error = Tier::parse("enormous").unwrap_err().to_string();
        assert!(error.contains("enormous"), "error: {error}");
        for tier in ["tiny", "small", "medium", "pathological", "large"] {
            assert!(error.contains(tier), "{tier} missing from: {error}");
        }
    }

    #[test]
    fn fingerprints_are_stable_and_react_to_every_field() {
        let recipe = tiny_recipe();
        assert_eq!(recipe.fingerprint(), recipe.fingerprint());

        let variants = [
            recipe,
            Recipe {
                tier: Tier::Small,
                ..recipe
            },
            Recipe {
                commits: recipe.commits + 1,
                ..recipe
            },
            Recipe {
                branches: recipe.branches + 1,
                ..recipe
            },
            Recipe {
                files: recipe.files + 1,
                ..recipe
            },
            Recipe {
                max_file_lines: recipe.max_file_lines + 1,
                ..recipe
            },
            Recipe {
                merge_fan: recipe.merge_fan + 1,
                ..recipe
            },
            Recipe {
                include_binary_blob: !recipe.include_binary_blob,
                ..recipe
            },
        ];

        let fingerprints: BTreeSet<u64> = variants.iter().map(Recipe::fingerprint).collect();
        assert_eq!(
            fingerprints.len(),
            variants.len(),
            "two recipes that differ share a fingerprint"
        );
    }

    #[test]
    fn tiers_live_in_directories_named_after_them() {
        let tiny = Tier::Tiny.recipe().path().unwrap();
        let small = Tier::Small.recipe().path().unwrap();
        assert_ne!(tiny, small);
        assert!(tiny.to_string_lossy().contains("tiny"));
    }

    #[test]
    fn generation_matches_the_recipe() {
        let dir = TempDir::new().unwrap();
        let recipe = tiny_recipe();
        let path = ensure_at(&recipe, &dir.path().join("corpus")).unwrap();

        let repo = Repository::open(&path).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_glob("refs/heads/*").unwrap();
        assert_eq!(walk.count(), recipe.commits, "commits reachable from refs");
        assert_eq!(
            repo.branches(Some(BranchType::Local)).unwrap().count(),
            recipe.branches
        );

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.author().name(), Some(AUTHOR_NAME));
        assert_eq!(
            head.time().seconds(),
            FIRST_COMMIT_TIME + recipe.commits as i64 - 1,
            "the clock advances one second per commit and never reads the wall clock"
        );

        let paths = all_paths(&repo);
        assert!(paths.contains("README.md"), "paths: {paths:?}");
        assert!(paths.contains("assets/texture.bin"), "paths: {paths:?}");

        // The working tree is checked out once at the end, so a benchmark that
        // asks for status does not see the whole repository as deleted.
        assert!(path.join("README.md").is_file());
        assert!(repo.statuses(None).unwrap().is_empty(), "status is clean");
    }

    #[test]
    fn the_biggest_file_is_exactly_as_long_as_the_recipe_says() {
        let dir = TempDir::new().unwrap();
        let recipe = Recipe {
            max_file_lines: 500,
            include_binary_blob: false,
            ..tiny_recipe()
        };
        let path = ensure_at(&recipe, &dir.path().join("corpus")).unwrap();

        let repo = Repository::open(&path).unwrap();
        let longest = all_blobs(&repo)
            .into_iter()
            .map(|oid| {
                let blob = repo.find_blob(oid).unwrap();
                blob.content().iter().filter(|byte| **byte == b'\n').count()
            })
            .max()
            .unwrap_or_default();
        assert_eq!(longest, recipe.max_file_lines);
    }

    #[test]
    fn generating_twice_produces_the_same_history() {
        let recipe = tiny_recipe();
        let first_dir = TempDir::new().unwrap();
        let second_dir = TempDir::new().unwrap();
        let first = ensure_at(&recipe, &first_dir.path().join("corpus")).unwrap();
        let second = ensure_at(&recipe, &second_dir.path().join("corpus")).unwrap();

        assert_eq!(head_oid(&first), head_oid(&second));
        assert_eq!(refs_of(&first), refs_of(&second));
    }

    #[test]
    fn a_built_corpus_is_reused_rather_than_rebuilt() {
        let dir = TempDir::new().unwrap();
        let recipe = tiny_recipe();
        let path = ensure_at(&recipe, &dir.path().join("corpus")).unwrap();
        let head = head_oid(&path);

        // A file generation never writes, inside `.git` so that restoring the
        // working tree does not remove it either: it survives only if the
        // corpus was reused rather than cleared and rebuilt.
        let sentinel = path.join(".git").join("sentinel.txt");
        std::fs::write(&sentinel, "kept").unwrap();
        ensure_at(&recipe, &path).unwrap();

        assert!(sentinel.is_file());
        assert_eq!(head_oid(&path), head);
    }

    #[test]
    fn a_corpus_a_run_dirtied_is_restored_before_it_is_handed_out_again() {
        let dir = TempDir::new().unwrap();
        let recipe = tiny_recipe();
        let path = ensure_at(&recipe, &dir.path().join("corpus")).unwrap();

        let readme = path.join("README.md");
        let pristine = std::fs::read_to_string(&readme).unwrap();

        // What a `dirty` step does, plus the untracked file and the staged
        // change a run that died partway through would leave behind.
        std::fs::write(
            &readme,
            format!(
                "{pristine}// touched by the perf harness
"
            ),
        )
        .unwrap();
        std::fs::write(path.join("scratch.txt"), "left over").unwrap();
        {
            let repo = Repository::open(&path).unwrap();
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("README.md")).unwrap();
            index.write().unwrap();
        }

        ensure_at(&recipe, &path).unwrap();

        assert_eq!(
            std::fs::read_to_string(&readme).unwrap(),
            pristine,
            "a tracked file the last run appended to is back to its committed content"
        );
        assert!(
            !path.join("scratch.txt").exists(),
            "an untracked file the last run left is gone"
        );
        let repo = Repository::open(&path).unwrap();
        assert!(repo.statuses(None).unwrap().is_empty(), "status is clean");
    }

    #[test]
    fn restoring_a_clean_corpus_leaves_its_history_alone() {
        let dir = TempDir::new().unwrap();
        let recipe = tiny_recipe();
        let path = ensure_at(&recipe, &dir.path().join("corpus")).unwrap();
        let head = head_oid(&path);
        let refs = refs_of(&path);

        restore(&path).unwrap();

        assert_eq!(head_oid(&path), head);
        assert_eq!(refs_of(&path), refs);
    }

    #[test]
    fn an_interrupted_corpus_is_rebuilt_rather_than_used() {
        let dir = TempDir::new().unwrap();
        let recipe = tiny_recipe();
        let path = ensure_at(&recipe, &dir.path().join("corpus")).unwrap();

        // What a run killed partway through leaves behind: objects on disk and
        // no marker.
        std::fs::remove_file(marker_path(&path)).unwrap();
        let sentinel = path.join("sentinel.txt");
        std::fs::write(&sentinel, "stale").unwrap();
        ensure_at(&recipe, &path).unwrap();

        assert!(
            !sentinel.exists(),
            "the stale tree should have been cleared"
        );
        assert!(marker_path(&path).is_file());
    }

    #[test]
    fn a_marker_from_another_recipe_does_not_count_as_complete() {
        let dir = TempDir::new().unwrap();
        let recipe = tiny_recipe();
        let path = ensure_at(&recipe, &dir.path().join("corpus")).unwrap();
        assert!(is_complete(&recipe, &path));

        let other = Recipe {
            commits: recipe.commits + 1,
            ..recipe
        };
        assert!(!is_complete(&other, &path));
    }

    #[test]
    fn the_pathological_tier_commits_crlf_unicode_paths_and_renames() {
        let dir = TempDir::new().unwrap();
        let recipe = pathological_recipe();
        let path = ensure_at(&recipe, &dir.path().join("corpus")).unwrap();
        let repo = Repository::open(&path).unwrap();

        let paths = all_paths(&repo);
        assert!(
            paths.iter().any(|path| !path.is_ascii()),
            "no non-ASCII path in {paths:?}"
        );
        assert!(
            paths.iter().any(|path| path.starts_with("legacy/chain_")),
            "no rename chain in {paths:?}"
        );

        let crlf = all_blobs(&repo).into_iter().any(|oid| {
            let blob = repo.find_blob(oid).unwrap();
            blob.content().windows(2).any(|pair| pair == b"\r\n")
        });
        assert!(crlf, "no CRLF file in the pathological tier");
    }

    #[test]
    fn merges_reach_the_recipe_s_fan() {
        let dir = TempDir::new().unwrap();
        let recipe = pathological_recipe();
        let path = ensure_at(&recipe, &dir.path().join("corpus")).unwrap();
        let repo = Repository::open(&path).unwrap();

        let mut walk = repo.revwalk().unwrap();
        walk.push_glob("refs/heads/*").unwrap();
        let widest = walk
            .map(|oid| repo.find_commit(oid.unwrap()).unwrap().parent_count())
            .max()
            .unwrap_or_default();
        assert_eq!(widest, recipe.merge_fan);
    }

    #[test]
    fn a_recipe_without_merges_stays_linear() {
        let dir = TempDir::new().unwrap();
        let recipe = Recipe {
            merge_fan: 0,
            branches: 1,
            ..tiny_recipe()
        };
        let path = ensure_at(&recipe, &dir.path().join("corpus")).unwrap();
        let repo = Repository::open(&path).unwrap();

        let mut walk = repo.revwalk().unwrap();
        walk.push_glob("refs/heads/*").unwrap();
        for oid in walk {
            let commit = repo.find_commit(oid.unwrap()).unwrap();
            assert!(commit.parent_count() <= 1, "{} is a merge", commit.id());
        }
    }

    #[test]
    fn the_number_generator_replays_its_stream() {
        let mut first = Prng::new(7);
        let mut second = Prng::new(7);
        let mut other = Prng::new(8);
        let a: Vec<u64> = (0..8).map(|_| first.next_u64()).collect();
        let b: Vec<u64> = (0..8).map(|_| second.next_u64()).collect();
        let c: Vec<u64> = (0..8).map(|_| other.next_u64()).collect();
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(Prng::new(1).below(0), 0);
    }
}
