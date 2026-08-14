//! [`HeapSize`] for types the app measures but neither it nor this crate owns.
//!
//! Rust's orphan rule means `rgitui_git` cannot implement a `rgitui_perf` trait
//! for a `git2` type, and `rgitui_diff` cannot implement one for a `gpui` type.
//! Since this crate owns the trait, the impls belong here — which also keeps
//! the coupling to `git2` and `gpui` out of [`crate::heap`], where the trait
//! and the census live.

use std::ops::Range;

use crate::heap::{Census, HeapSize};

/// A git object id is 20 or 32 inline bytes with no indirection.
impl HeapSize for git2::Oid {
    fn heap_size(&self, _census: &mut Census) -> usize {
        0
    }
}

/// A range is two integers.
impl<T: HeapSize> HeapSize for Range<T> {
    fn heap_size(&self, census: &mut Census) -> usize {
        self.start.heap_size(census) + self.end.heap_size(census)
    }
}

/// A `SharedString` is either a `&'static str` this process never allocated or
/// an `Arc<str>` that clones share, so it is charged through the census's
/// shared-payload path keyed on the text buffer's address.
///
/// Doing this by pointer rather than by length is what keeps the diff viewer's
/// numbers honest: it stores every line once as a unified row and again as a
/// side-by-side row, and those two rows hold clones of the *same* buffer.
/// Charging `len()` at each site would report double the text the app is
/// actually holding, and the resulting "finding" would send someone
/// deduplicating storage that was never duplicated.
impl HeapSize for gpui::SharedString {
    fn heap_size(&self, census: &mut Census) -> usize {
        let text: &str = self.as_ref();
        if text.is_empty() {
            return 0;
        }
        census
            .visit_shared(text.as_ptr() as usize, &())
            .map(|_| text.len())
            .unwrap_or(0)
    }
}

/// The unit type owns nothing. It exists here so a shared payload whose size is
/// known to the caller can still be routed through
/// [`Census::visit_shared`], which needs *something* to walk.
impl HeapSize for () {
    fn heap_size(&self, _census: &mut Census) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shared_string_is_charged_once_however_many_clones_hold_it() {
        let mut census = Census::new();
        let original = gpui::SharedString::from("a line of source code".to_string());
        let clone = original.clone();

        let first = original.heap_size(&mut census);
        let second = clone.heap_size(&mut census);

        assert_eq!(first, "a line of source code".len());
        assert_eq!(
            second, 0,
            "a clone shares the buffer and must not be charged"
        );
    }

    #[test]
    fn distinct_shared_strings_are_charged_separately() {
        let mut census = Census::new();
        let first = gpui::SharedString::from("first".to_string());
        let second = gpui::SharedString::from("second".to_string());

        assert_eq!(first.heap_size(&mut census), "first".len());
        assert_eq!(second.heap_size(&mut census), "second".len());
    }

    #[test]
    fn an_empty_shared_string_costs_nothing() {
        let mut census = Census::new();
        assert_eq!(gpui::SharedString::default().heap_size(&mut census), 0);
    }

    #[test]
    fn a_range_owns_nothing() {
        let mut census = Census::new();
        assert_eq!((0usize..10usize).heap_size(&mut census), 0);
    }
}
