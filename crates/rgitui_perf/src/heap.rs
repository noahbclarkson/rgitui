//! What the heap is made of, labelled by feature rather than by call stack.
//!
//! An allocation-site profiler (see the `dhat-heap` feature) answers "which
//! line of code allocated this". That is the wrong question when deciding what
//! to cache less of: `String::from` is always the top frame and never the
//! culprit. This module answers "which *feature* is holding this memory" —
//!
//! ```text
//! tab[0]/graph/commits               48.2 MB  (12,431 commits)
//! tab[0]/diff_viewer/display_cache   31.7 MB  (37 entries)
//! tab[0]/caches/diff                120.4 MB  (200 entries)
//! ```
//!
//! — which is directly actionable. The two layers are complementary, and the
//! harness cross-checks them: a large gap between the census total and the
//! process's private bytes is itself a finding.
//!
//! # Shared ownership
//!
//! rgitui `Arc`s its large snapshots deliberately, so the same
//! `Arc<Vec<CommitInfo>>` is reachable from both `GitProject` and `GraphView`.
//! A naive walk would count it twice and the report would overstate the heap by
//! more than the thing being investigated. [`Census`] therefore records the
//! pointer identity of every `Arc` it visits and attributes the payload to the
//! first path that reaches it.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// Heap bytes owned by a value, excluding the value's own inline size.
///
/// Implement this next to the type it describes, not here: this crate sits
/// below everything it measures and must not depend on the crates that own the
/// data.
///
/// # Contract
///
/// - Return only memory reachable *and owned* through `self`. A `&T` or a
///   `Weak<T>` borrows and must contribute nothing.
/// - Count spare capacity, not just length: a `Vec` that grew and shrank still
///   holds its buffer, and hiding that would hide a real cost.
/// - Route every `Arc`, `Rc` and shared handle through [`Census::visit_shared`]
///   so shared payloads are counted once.
pub trait HeapSize {
    /// Bytes this value owns on the heap, excluding `size_of::<Self>()`.
    fn heap_size(&self, census: &mut Census) -> usize;

    /// Number of logical items this value represents, for reports that are
    /// clearer with a count beside the byte total ("200 entries", "12,431
    /// commits"). Defaults to no count.
    fn heap_count(&self) -> Option<usize> {
        None
    }
}

/// One labelled node in the census tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CensusNode {
    /// Slash-separated path identifying what this is, e.g.
    /// `tab[0]/diff_viewer/display_cache`.
    pub path: String,
    /// Heap bytes attributed to this node, including its children.
    pub bytes: usize,
    /// Logical item count, when the node has a meaningful one.
    pub count: Option<usize>,
}

/// Accumulates a labelled tree of heap attributions.
///
/// Built by walking the app's roots and calling [`Census::enter`] for each named
/// sub-tree. Shared payloads are deduplicated by pointer identity.
#[derive(Debug, Default)]
pub struct Census {
    nodes: Vec<CensusNode>,
    path: Vec<String>,
    seen_shared: HashSet<usize>,
}

impl Census {
    /// Starts an empty census.
    pub fn new() -> Self {
        Self::default()
    }

    /// Measures `value` under `label`, nesting it beneath any enclosing
    /// [`Census::enter`] scopes, and records a node for it.
    ///
    /// Returns the bytes attributed, so callers can roll their own subtotals.
    pub fn enter<T: HeapSize + ?Sized>(&mut self, label: &str, value: &T) -> usize {
        self.path.push(label.to_string());
        let bytes = value.heap_size(self);
        let path = self.path.join("/");
        self.path.pop();
        self.nodes.push(CensusNode {
            path,
            bytes,
            count: value.heap_count(),
        });
        bytes
    }

    /// Measures a shared payload exactly once.
    ///
    /// `id` must be the address of the shared allocation (`Arc::as_ptr(&arc) as
    /// usize`). The first path to reach it is charged for it and gets
    /// `Some(bytes)`; later paths get `None`, so totals stay truthful even
    /// though the graph is not a tree.
    ///
    /// The answer is an `Option` rather than a plain `0` because the caller has
    /// its own bytes to add: an `Arc` also owns the payload's inline size,
    /// which lives in the shared allocation. A zero could not be told apart
    /// from a first visit to a payload that happens to own nothing further out,
    /// and the inline size would then be charged once per clone — the exact
    /// double count this method exists to prevent.
    pub fn visit_shared<T: HeapSize + ?Sized>(&mut self, id: usize, value: &T) -> Option<usize> {
        if !self.seen_shared.insert(id) {
            return None;
        }
        Some(value.heap_size(self))
    }

    /// The recorded nodes, largest first.
    pub fn into_nodes(mut self) -> Vec<CensusNode> {
        self.nodes.sort_by_key(|node| std::cmp::Reverse(node.bytes));
        self.nodes
    }

    /// Total bytes across every top-level node.
    ///
    /// Only depth-1 paths are summed; nested nodes are already included in
    /// their parent's total and adding them again would double count.
    pub fn total_bytes(&self) -> usize {
        self.nodes
            .iter()
            .filter(|node| !node.path.contains('/'))
            .map(|node| node.bytes)
            .sum()
    }
}

/// Scalars own nothing past their own inline bytes, which the holder already
/// accounts for, so they report zero. They still need the impl: a struct of
/// counters, offsets and flags has to be walkable field by field without its
/// author first deciding which fields are "the interesting ones", because that
/// decision is exactly the guess the census exists to replace.
macro_rules! impl_heap_size_for_scalar {
    ($($ty:ty),* $(,)?) => {
        $(
            impl HeapSize for $ty {
                fn heap_size(&self, _census: &mut Census) -> usize {
                    0
                }
            }
        )*
    };
}

impl_heap_size_for_scalar!(
    bool, char, u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64,
);

/// A `String` is charged its capacity, not its length. A name buffer that grew
/// to a megabyte and was truncated to twelve characters is still holding the
/// megabyte, and that is precisely the kind of cost a census is asked to find.
impl HeapSize for String {
    fn heap_size(&self, _census: &mut Census) -> usize {
        self.capacity()
    }
}

/// A `str` is its bytes, so those bytes are its inline size and belong to
/// whatever owns the allocation — a `String`, a `Box<str>`, an `Arc<str>`.
/// Charging them here as well would count every shared string twice.
impl HeapSize for str {
    fn heap_size(&self, _census: &mut Census) -> usize {
        0
    }
}

/// `PathBuf` wraps an `OsString`, so the same capacity-not-length rule applies.
/// Repository paths are held per file in status and diff snapshots, which makes
/// them one of the larger string populations in the app.
impl HeapSize for PathBuf {
    fn heap_size(&self, _census: &mut Census) -> usize {
        self.capacity()
    }
}

/// A `Path` is an unsized view of bytes owned elsewhere, like [`str`].
impl HeapSize for Path {
    fn heap_size(&self, _census: &mut Census) -> usize {
        0
    }
}

/// A `Vec` owns two separable things and an honest report names both: the
/// buffer it allocated (`capacity * size_of::<T>()`) and whatever the elements
/// point at further out. Counting only the buffer would price a million-name
/// `Vec<String>` at 24 MB of pointers and hide the names themselves; counting
/// only the elements would hide a vector that was drained but never shrunk.
impl<T: HeapSize> HeapSize for Vec<T> {
    fn heap_size(&self, census: &mut Census) -> usize {
        self.capacity() * size_of::<T>() + self.as_slice().heap_size(census)
    }

    fn heap_count(&self) -> Option<usize> {
        Some(self.len())
    }
}

/// A slice does not own its buffer — the `Vec`, `Box<[T]>` or `Arc<[T]>` behind
/// it does — so only what the elements own further out is charged here.
impl<T: HeapSize> HeapSize for [T] {
    fn heap_size(&self, census: &mut Census) -> usize {
        self.iter().map(|item| item.heap_size(census)).sum()
    }

    fn heap_count(&self) -> Option<usize> {
        Some(self.len())
    }
}

/// An `Option`'s own discriminant and payload are inline in whatever holds it,
/// so only the payload's outward-facing memory is charged.
impl<T: HeapSize> HeapSize for Option<T> {
    fn heap_size(&self, census: &mut Census) -> usize {
        self.as_ref().map_or(0, |value| value.heap_size(census))
    }

    fn heap_count(&self) -> Option<usize> {
        self.as_ref().and_then(HeapSize::heap_count)
    }
}

/// A `Box` is a private allocation, so unlike an `Arc` it needs no dedup: the
/// payload's inline size is charged unconditionally, plus whatever the payload
/// owns beyond it. `size_of_val` rather than `size_of` so `Box<[T]>` and
/// `Box<str>` report the length they actually allocated.
impl<T: HeapSize + ?Sized> HeapSize for Box<T> {
    fn heap_size(&self, census: &mut Census) -> usize {
        size_of_val(&**self) + (**self).heap_size(census)
    }

    fn heap_count(&self) -> Option<usize> {
        (**self).heap_count()
    }
}

/// The impl that makes the whole report trustworthy.
///
/// rgitui hands the same `Arc<Vec<CommitInfo>>` to `GitProject` and
/// `GraphView` on purpose. Charging the payload to the first path that reaches
/// it — and charging the later ones nothing at all, header included — keeps the
/// total equal to what the process actually allocated. A repeat visit must
/// return a bare zero rather than the inline size, since that size lives in the
/// shared allocation and was already counted by the first visitor.
impl<T: HeapSize + ?Sized> HeapSize for Arc<T> {
    fn heap_size(&self, census: &mut Census) -> usize {
        let id = Arc::as_ptr(self) as *const () as usize;
        match census.visit_shared(id, &**self) {
            Some(payload) => size_of_val(&**self) + payload,
            None => 0,
        }
    }

    fn heap_count(&self) -> Option<usize> {
        (**self).heap_count()
    }
}

/// Single-threaded sharing is deduplicated exactly like [`Arc`]; the two
/// pointer families cannot collide because a live allocation has one address.
impl<T: HeapSize + ?Sized> HeapSize for Rc<T> {
    fn heap_size(&self, census: &mut Census) -> usize {
        let id = Rc::as_ptr(self) as *const () as usize;
        match census.visit_shared(id, &**self) {
            Some(payload) => size_of_val(&**self) + payload,
            None => 0,
        }
    }

    fn heap_count(&self) -> Option<usize> {
        (**self).heap_count()
    }
}

/// A borrow owns nothing. The pointee is charged to whoever holds it, and
/// charging it again here would inflate the report by however much of the
/// structure is cross-referenced — the same failure mode as counting a shared
/// `Arc` twice. The `T: HeapSize` bound keeps the impl from quietly swallowing
/// types nobody has taught the census to measure.
impl<T: HeapSize + ?Sized> HeapSize for &T {
    fn heap_size(&self, _census: &mut Census) -> usize {
        0
    }
}

/// A `Weak` handle keeps no payload alive, so it contributes nothing even when
/// it happens to be upgradable at census time.
impl<T: HeapSize + ?Sized> HeapSize for std::sync::Weak<T> {
    fn heap_size(&self, _census: &mut Census) -> usize {
        0
    }
}

/// The single-threaded counterpart of the [`std::sync::Weak`] impl.
impl<T: HeapSize + ?Sized> HeapSize for std::rc::Weak<T> {
    fn heap_size(&self, _census: &mut Census) -> usize {
        0
    }
}

/// Bytes a hashbrown table reserves to hold `capacity` entries of type `E`.
///
/// `std`'s maps keep the entries in one array with a parallel byte of control
/// metadata per bucket. The figure is deliberately approximate: hashbrown
/// rounds the bucket array up to a power of two and reports `capacity()` as
/// 7/8 of it, so the live allocation is roughly an eighth larger than this.
/// Erring low is the right direction for a report whose job is to point at
/// memory it can name, and an eighth of a bucket array is noise beside the
/// payloads the entries themselves own.
fn table_bytes<E>(capacity: usize) -> usize {
    capacity * (size_of::<E>() + 1)
}

/// hashbrown stores `(K, V)` pairs, so the per-entry cost is the tuple's size
/// with its padding, not the two sizes added up. On top of the table come the
/// keys' and values' own allocations, which for a `HashMap<String, FileStatus>`
/// is where nearly all of the memory actually is.
impl<K: HeapSize, V: HeapSize, S> HeapSize for HashMap<K, V, S> {
    fn heap_size(&self, census: &mut Census) -> usize {
        let mut bytes = table_bytes::<(K, V)>(self.capacity());
        for (key, value) in self {
            bytes += key.heap_size(census) + value.heap_size(census);
        }
        bytes
    }

    fn heap_count(&self) -> Option<usize> {
        Some(self.len())
    }
}

/// A set is the same table with no value half.
impl<T: HeapSize, S> HeapSize for HashSet<T, S> {
    fn heap_size(&self, census: &mut Census) -> usize {
        let mut bytes = table_bytes::<T>(self.capacity());
        for item in self {
            bytes += item.heap_size(census);
        }
        bytes
    }

    fn heap_count(&self) -> Option<usize> {
        Some(self.len())
    }
}

/// A pair is charged as its halves. Needed because maps of tuples are how
/// several of the app's caches are keyed — `HashMap<PathBuf, (u64, Status)>` in
/// the file watcher, for one — and without this impl the payload beside the
/// fingerprint would silently drop out of the report.
impl<A: HeapSize, B: HeapSize> HeapSize for (A, B) {
    fn heap_size(&self, census: &mut Census) -> usize {
        self.0.heap_size(census) + self.1.heap_size(census)
    }
}

/// A `BTreeMap` allocates fixed-size nodes holding up to eleven entries each
/// rather than one contiguous table, so it is priced per entry at the node
/// slot's own width. The figure understates by however full the nodes happen to
/// be — a tree built by ascending insertion packs them, a randomly built one
/// leaves them around 70% full — which is the right direction for a report
/// whose job is to name memory it can account for.
impl<K: HeapSize, V: HeapSize> HeapSize for BTreeMap<K, V> {
    fn heap_size(&self, census: &mut Census) -> usize {
        let mut bytes = self.len() * size_of::<(K, V)>();
        for (key, value) in self {
            bytes += key.heap_size(census) + value.heap_size(census);
        }
        bytes
    }

    fn heap_count(&self) -> Option<usize> {
        Some(self.len())
    }
}

/// The same tree with no value half.
impl<T: HeapSize> HeapSize for BTreeSet<T> {
    fn heap_size(&self, census: &mut Census) -> usize {
        let mut bytes = self.len() * size_of::<T>();
        for item in self {
            bytes += item.heap_size(census);
        }
        bytes
    }

    fn heap_count(&self) -> Option<usize> {
        Some(self.len())
    }
}

/// A `Mutex` holds its payload inline, so only the payload's outward memory is
/// charged.
///
/// A poisoned lock reports nothing: the payload is unreachable without asserting
/// over a panic that already happened, and a census is not the place to do that.
/// The count says which case it was, so a skipped node cannot be mistaken for an
/// empty one.
impl<T: HeapSize> HeapSize for Mutex<T> {
    fn heap_size(&self, census: &mut Census) -> usize {
        match self.lock() {
            Ok(value) => value.heap_size(census),
            Err(_) => 0,
        }
    }

    fn heap_count(&self) -> Option<usize> {
        self.lock().ok().and_then(|value| value.heap_count())
    }
}

/// A `VecDeque` is a ring buffer over one allocation, so it is priced like a
/// `Vec`: the whole buffer, then whatever the live elements own.
impl<T: HeapSize> HeapSize for VecDeque<T> {
    fn heap_size(&self, census: &mut Census) -> usize {
        let mut bytes = self.capacity() * size_of::<T>();
        for item in self {
            bytes += item.heap_size(census);
        }
        bytes
    }

    fn heap_count(&self) -> Option<usize> {
        Some(self.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A commit-shaped leaf: one owned string, like the real `CommitInfo`.
    struct Commit {
        message: String,
    }

    impl HeapSize for Commit {
        fn heap_size(&self, census: &mut Census) -> usize {
            self.message.heap_size(census)
        }
    }

    /// Mirrors the real sharing: `GitProject` and `GraphView` hold clones of
    /// one snapshot, and each labels it under its own path.
    struct GitProject {
        commits: Arc<Vec<Commit>>,
    }

    impl HeapSize for GitProject {
        fn heap_size(&self, census: &mut Census) -> usize {
            census.enter("commits", &self.commits)
        }
    }

    struct GraphView {
        commits: Arc<Vec<Commit>>,
    }

    impl HeapSize for GraphView {
        fn heap_size(&self, census: &mut Census) -> usize {
            census.enter("commits", &self.commits)
        }
    }

    fn commit_snapshot(count: usize) -> Arc<Vec<Commit>> {
        Arc::new(
            (0..count)
                .map(|index| Commit {
                    message: format!("commit {index:04} touched the parser"),
                })
                .collect(),
        )
    }

    /// The payload a snapshot owns: the `Vec` header inside the `Arc`, the
    /// element buffer, and every message's own allocation.
    fn snapshot_bytes(commits: &Arc<Vec<Commit>>) -> usize {
        size_of::<Vec<Commit>>()
            + commits.capacity() * size_of::<Commit>()
            + commits
                .iter()
                .map(|commit| commit.message.capacity())
                .sum::<usize>()
    }

    #[test]
    fn strings_and_vectors_report_capacity_rather_than_length() {
        let mut census = Census::new();

        let mut text = String::with_capacity(128);
        text.push_str("short");
        assert_eq!(text.heap_size(&mut census), 128);

        let mut numbers: Vec<u32> = Vec::with_capacity(64);
        numbers.push(1);
        assert_eq!(numbers.heap_size(&mut census), 64 * size_of::<u32>());
        assert_eq!(numbers.heap_count(), Some(1));
    }

    #[test]
    fn a_vector_of_strings_counts_the_buffer_and_the_strings() {
        let mut census = Census::new();
        let words = vec![String::from("alpha"), String::from("beta")];

        let buffer = words.capacity() * size_of::<String>();
        let contents: usize = words.iter().map(String::capacity).sum();

        assert!(contents > 0);
        assert_eq!(words.heap_size(&mut census), buffer + contents);
    }

    #[test]
    fn empty_containers_own_nothing() {
        let mut census = Census::new();

        assert_eq!(String::new().heap_size(&mut census), 0);
        assert_eq!(Vec::<u8>::new().heap_size(&mut census), 0);
        assert_eq!(VecDeque::<u8>::new().heap_size(&mut census), 0);
        assert_eq!(HashMap::<u32, String>::new().heap_size(&mut census), 0);
        assert_eq!(HashSet::<u32>::new().heap_size(&mut census), 0);
        assert_eq!(Option::<String>::None.heap_size(&mut census), 0);
    }

    #[test]
    fn a_map_counts_its_table_and_what_its_entries_point_at() {
        let mut census = Census::new();
        let mut refs: HashMap<String, Vec<u32>> = HashMap::new();
        refs.insert("origin/main".to_string(), vec![1, 2, 3]);

        let table = refs.capacity() * (size_of::<(String, Vec<u32>)>() + 1);
        let entries: usize = refs
            .iter()
            .map(|(name, ids)| name.capacity() + ids.capacity() * size_of::<u32>())
            .sum();

        assert!(entries > 0);
        assert_eq!(refs.heap_size(&mut census), table + entries);
        assert_eq!(refs.heap_count(), Some(1));
    }

    #[test]
    fn a_pair_is_charged_for_both_of_its_halves() {
        let mut census = Census::new();
        let pair = (7u64, String::with_capacity(96));

        assert_eq!(pair.heap_size(&mut census), 96);
    }

    #[test]
    fn an_ordered_map_counts_its_nodes_and_what_they_point_at() {
        let mut census = Census::new();
        let mut themes: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        themes.insert("base16-ocean.dark".to_string(), vec![1, 2]);

        let nodes = size_of::<(String, Vec<u32>)>();
        let entries = "base16-ocean.dark".len() + 2 * size_of::<u32>();

        assert_eq!(themes.heap_size(&mut census), nodes + entries);
        assert_eq!(themes.heap_count(), Some(1));
        assert_eq!(BTreeMap::<u32, u32>::new().heap_size(&mut census), 0);
    }

    #[test]
    fn an_ordered_set_is_the_same_tree_without_values() {
        let mut census = Census::new();
        let ids: BTreeSet<usize> = (0..4).collect();

        assert_eq!(ids.heap_size(&mut census), 4 * size_of::<usize>());
        assert_eq!(ids.heap_count(), Some(4));
    }

    #[test]
    fn a_mutex_is_charged_for_what_it_guards() {
        let mut census = Census::new();
        let guarded = Mutex::new(String::with_capacity(256));

        assert_eq!(guarded.heap_size(&mut census), 256);
    }

    #[test]
    fn a_poisoned_mutex_reports_no_count_rather_than_an_empty_one() {
        let guarded = Arc::new(Mutex::new(vec![String::with_capacity(64)]));
        let poisoner = Arc::clone(&guarded);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().expect("the lock is not poisoned yet");
            panic!("poison the lock");
        })
        .join();

        let mut census = Census::new();
        assert!(guarded.lock().is_err());
        assert_eq!((*guarded).heap_size(&mut census), 0);
        assert_eq!((*guarded).heap_count(), None);
    }

    #[test]
    fn a_box_carries_its_payloads_inline_bytes_too() {
        let mut census = Census::new();
        let boxed: Box<String> = Box::new(String::with_capacity(32));

        assert_eq!(boxed.heap_size(&mut census), size_of::<String>() + 32);
    }

    #[test]
    fn a_borrow_is_charged_to_its_owner_not_to_the_borrower() {
        let mut census = Census::new();
        let owned = [String::with_capacity(64), String::with_capacity(64)];
        let borrowed: Vec<&String> = owned.iter().collect();

        assert_eq!(
            borrowed.heap_size(&mut census),
            borrowed.capacity() * size_of::<&String>()
        );
    }

    #[test]
    fn visiting_a_shared_id_twice_reports_nothing_the_second_time() {
        let mut census = Census::new();
        let text = String::with_capacity(48);

        assert_eq!(census.visit_shared(1, &text), Some(48));
        assert_eq!(census.visit_shared(1, &text), None);
        assert_eq!(census.visit_shared(2, &text), Some(48));
    }

    #[test]
    fn a_shared_snapshot_is_charged_to_the_first_path_only() {
        let commits = commit_snapshot(32);
        let project = GitProject {
            commits: Arc::clone(&commits),
        };
        let graph = GraphView {
            commits: Arc::clone(&commits),
        };

        let mut census = Census::new();
        let project_bytes = census.enter("project", &project);
        let graph_bytes = census.enter("graph", &graph);

        let payload = snapshot_bytes(&commits);
        assert!(payload > 0);
        assert_eq!(project_bytes, payload);
        assert_eq!(graph_bytes, 0);
        assert_eq!(census.total_bytes(), payload);
    }

    #[test]
    fn a_second_handle_adds_no_header_of_its_own() {
        let mut census = Census::new();
        let shared = Rc::new(String::with_capacity(64));
        let clone = Rc::clone(&shared);

        assert_eq!(shared.heap_size(&mut census), size_of::<String>() + 64);
        assert_eq!(clone.heap_size(&mut census), 0);
    }

    #[test]
    fn entering_a_scope_nests_the_path_and_records_a_node() {
        let mut census = Census::new();
        census.enter(
            "tab[0]",
            &GitProject {
                commits: commit_snapshot(4),
            },
        );

        let nodes = census.into_nodes();
        let nested = nodes
            .iter()
            .find(|node| node.path == "tab[0]/commits")
            .expect("the nested scope is recorded under its parent's path");

        assert!(nodes.iter().any(|node| node.path == "tab[0]"));
        assert_eq!(nested.count, Some(4));
    }

    #[test]
    fn the_total_counts_top_level_nodes_only() {
        let mut census = Census::new();
        let project = GitProject {
            commits: commit_snapshot(8),
        };
        let bytes = census.enter("tab[0]", &project);

        assert_eq!(census.total_bytes(), bytes);

        let nodes = census.into_nodes();
        assert_eq!(nodes.len(), 2);
        assert_eq!(
            nodes.iter().map(|node| node.bytes).sum::<usize>(),
            bytes * 2,
            "the nested node repeats its parent's bytes, so a flat sum would double count"
        );
    }

    #[test]
    fn nodes_come_back_largest_first() {
        let mut census = Census::new();
        census.enter("small", &String::with_capacity(16));
        census.enter("large", &String::with_capacity(1024));
        census.enter("medium", &String::with_capacity(256));

        let paths: Vec<String> = census
            .into_nodes()
            .into_iter()
            .map(|node| node.path)
            .collect();

        assert_eq!(paths, ["large", "medium", "small"]);
    }
}
