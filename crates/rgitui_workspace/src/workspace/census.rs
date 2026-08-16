//! Walking the workspace into a labelled heap census.
//!
//! Labels are the product here as much as the byte counts are. A total tells
//! you the app is using 600 MB; `tab[1]/caches/diff = 412 MB (200 entries)`
//! tells you which bound to change. Paths are therefore kept short and stable,
//! because a comparison between two runs matches nodes by path and a renamed
//! label reads as a node that vanished.

use gpui::App;
use rgitui_perf::{Census, HeapSize};

use super::{ProjectTab, ViewCacheEntry, ViewCacheKey, ViewCaches, Workspace};
use crate::cache::LruCache;

impl Workspace {
    /// Records every open tab's retained memory.
    pub(crate) fn census(&self, census: &mut Census, cx: &App) {
        for (index, tab) in self.tabs.iter().enumerate() {
            let label = format!("tab[{index}]");
            census.enter(&label, &TabCensus { tab, cx });
        }
    }

    /// Whether the workspace has no git operation running.
    ///
    /// Used by scenario `wait_for` steps so a measurement starts after the work
    /// it is measuring has finished, rather than partway through it.
    pub(crate) fn is_operation_idle(&self) -> bool {
        self.operations.active_operations.is_empty()
            && self.operations.active_git_operation.is_none()
    }
}

/// Adapter that lets a tab be walked by the census, which needs `&App` to read
/// the entities a tab holds and cannot get one through [`HeapSize`] alone.
struct TabCensus<'a> {
    tab: &'a ProjectTab,
    cx: &'a App,
}

impl HeapSize for TabCensus<'_> {
    fn heap_size(&self, census: &mut Census) -> usize {
        let mut total = 0;

        total += census.enter(
            "graph",
            &GraphCensus {
                tab: self.tab,
                cx: self.cx,
            },
        );
        total += census.enter(
            "diff_viewer",
            &DiffCensus {
                tab: self.tab,
                cx: self.cx,
            },
        );
        total += census.enter("caches", &self.tab.caches);

        // The panels each hold their own copy of whatever the project handed
        // them — a blame's per-line authorship, a reflog, a search result set —
        // and until these were walked the census could only see the graph, the
        // diff viewer and the caches, which is how it came to report about a
        // tenth of the heap that dhat measured.
        total += census.enter(
            "panels",
            &PanelCensus {
                tab: self.tab,
                cx: self.cx,
            },
        );
        total += census.enter("name", &self.tab.name);

        total
    }
}

/// The per-tab panels, each charged under its own label.
struct PanelCensus<'a> {
    tab: &'a ProjectTab,
    cx: &'a App,
}

impl HeapSize for PanelCensus<'_> {
    fn heap_size(&self, census: &mut Census) -> usize {
        let mut total = 0;

        total += census.enter("sidebar", &EntityCensus(self.tab.sidebar.read(self.cx)));
        total += census.enter("detail", &EntityCensus(self.tab.detail_panel.read(self.cx)));
        total += census.enter("blame", &EntityCensus(self.tab.blame_view.read(self.cx)));
        total += census.enter(
            "file_history",
            &EntityCensus(self.tab.file_history_view.read(self.cx)),
        );
        total += census.enter("reflog", &EntityCensus(self.tab.reflog_view.read(self.cx)));
        total += census.enter(
            "search",
            &EntityCensus(self.tab.global_search_view.read(self.cx)),
        );
        total += census.enter(
            "stashes",
            &EntityCensus(self.tab.stashes_panel.read(self.cx)),
        );

        total
    }
}

/// Bridges a panel's inherent `census` method to [`HeapSize`], so that
/// `Census::enter` can label it without every panel implementing the trait.
struct EntityCensus<'a, T>(&'a T);

macro_rules! panel_census {
    ($($panel:ty),+ $(,)?) => {
        $(
            impl HeapSize for EntityCensus<'_, $panel> {
                fn heap_size(&self, census: &mut Census) -> usize {
                    self.0.census(census)
                }
            }
        )+
    };
}

panel_census!(
    crate::Sidebar,
    crate::DetailPanel,
    crate::BlameView,
    crate::FileHistoryView,
    crate::ReflogView,
    crate::GlobalSearchView,
    crate::StashesPanel,
);

/// The graph view's retained rows and commit snapshot.
struct GraphCensus<'a> {
    tab: &'a ProjectTab,
    cx: &'a App,
}

impl HeapSize for GraphCensus<'_> {
    fn heap_size(&self, census: &mut Census) -> usize {
        self.tab.graph.read(self.cx).census(census)
    }
}

/// The diff viewer's prepared rows and display cache.
struct DiffCensus<'a> {
    tab: &'a ProjectTab,
    cx: &'a App,
}

impl HeapSize for DiffCensus<'_> {
    fn heap_size(&self, census: &mut Census) -> usize {
        self.tab.diff_viewer.read(self.cx).census(census)
    }
}

/// A cache key owns two paths and an optional commit id.
impl HeapSize for ViewCacheKey {
    fn heap_size(&self, census: &mut Census) -> usize {
        self.repo_path.heap_size(census)
            + self.file_path.heap_size(census)
            + self.commit_id.heap_size(census)
    }
}

impl<T: HeapSize> HeapSize for ViewCacheEntry<T> {
    fn heap_size(&self, census: &mut Census) -> usize {
        match self {
            ViewCacheEntry::Loading => 0,
            ViewCacheEntry::Ready(value) => value.heap_size(census),
            ViewCacheEntry::Unavailable(reason) => reason.heap_size(census),
        }
    }

    fn heap_count(&self) -> Option<usize> {
        match self {
            ViewCacheEntry::Ready(value) => value.heap_count(),
            _ => None,
        }
    }
}

/// The three per-tab caches, each reported separately.
///
/// Splitting them matters because they are bounded very differently: blame and
/// history hold 24 entries each, while the diff cache holds 200 whole
/// `CommitDiff`s — every file of every commit visited, with every line as an
/// owned `String`. Rolling the three into one number would hide which bound is
/// the expensive one.
impl HeapSize for ViewCaches {
    fn heap_size(&self, census: &mut Census) -> usize {
        let mut total = 0;

        if let Ok(blame) = self.blame.lock() {
            total += census.enter("blame", &*blame);
        }
        if let Ok(history) = self.history.lock() {
            total += census.enter("history", &*history);
        }
        if let Ok(diff) = self.diff.lock() {
            total += census.enter("diff", &*diff);
        }

        total
    }
}

impl<K, V> HeapSize for LruCache<K, V>
where
    K: std::hash::Hash + Eq + Clone + HeapSize,
    V: HeapSize,
{
    fn heap_size(&self, census: &mut Census) -> usize {
        self.census_entries(census)
    }

    fn heap_count(&self) -> Option<usize> {
        Some(self.census_len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cache_entry_charges_only_what_its_variant_holds() {
        let mut census = Census::new();

        assert_eq!(
            ViewCacheEntry::<String>::Loading.heap_size(&mut census),
            0,
            "a reserved slot holds no data yet"
        );
        assert_eq!(
            ViewCacheEntry::<String>::Unavailable("no such file".to_string())
                .heap_size(&mut census),
            "no such file".len()
        );
        assert_eq!(
            ViewCacheEntry::Ready("blame output".to_string()).heap_size(&mut census),
            "blame output".len()
        );
    }

    #[test]
    fn a_cache_reports_its_entries_and_their_contents() {
        let mut cache: LruCache<String, String> = LruCache::new(4);
        cache.insert("a".to_string(), "first".to_string());
        cache.insert("b".to_string(), "second".to_string());

        let mut census = Census::new();
        let bytes = cache.heap_size(&mut census);

        assert_eq!(cache.heap_count(), Some(2));
        assert!(
            bytes >= "a".len() + "first".len() + "b".len() + "second".len(),
            "keys and values must both be charged, got {bytes}"
        );
    }

    #[test]
    fn an_empty_cache_costs_nothing_it_has_not_allocated() {
        let cache: LruCache<String, String> = LruCache::new(4);
        let mut census = Census::new();
        assert_eq!(cache.heap_count(), Some(0));
        assert_eq!(cache.heap_size(&mut census), 0);
    }
}
