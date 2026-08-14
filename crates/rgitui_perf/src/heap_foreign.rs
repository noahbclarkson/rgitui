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

/// A `SharedString` wraps a small-string-optimised type: short text lives
/// inline in the value itself, longer text in an allocation that clones share.
/// Only the second kind is on the heap, and only it can be shared.
///
/// The two are told apart by asking whether the text bytes lie inside this
/// value's own footprint. That reads as a trick, and the alternative is worse:
/// hard-coding the upstream inline threshold would silently start
/// over-reporting the day it changes, which is exactly what happened to the
/// previous version of this impl when `SharedString` became a `SmolStr`.
///
/// Out-of-line text is charged through the census's shared-payload path, keyed
/// on the buffer address rather than on length. That is what keeps the diff
/// viewer's numbers honest: it holds every line once as a unified row and again
/// as a side-by-side row, and those two rows hold clones of one buffer.
/// Charging `len()` at both sites would report double the text the app actually
/// holds, and the resulting finding would send someone deduplicating storage
/// that was never duplicated.
impl HeapSize for gpui::SharedString {
    fn heap_size(&self, census: &mut Census) -> usize {
        let text: &str = self.as_ref();
        if text.is_empty() {
            return 0;
        }

        let inline_start = std::ptr::from_ref(self) as usize;
        let text_start = text.as_ptr() as usize;
        let is_inline = text_start >= inline_start
            && text_start < inline_start + std::mem::size_of::<gpui::SharedString>();
        if is_inline {
            return 0;
        }

        census
            .visit_shared(text_start, &())
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

    /// Comfortably past any plausible inline threshold, so this text is
    /// genuinely heap-allocated and genuinely shared between clones.
    const LONG: &str = "a line of source code long enough that it cannot possibly be stored inline";

    #[test]
    fn a_shared_string_is_charged_once_however_many_clones_hold_it() {
        let mut census = Census::new();
        let original = gpui::SharedString::from(LONG.to_string());
        let clone = original.clone();

        let first = original.heap_size(&mut census);
        let second = clone.heap_size(&mut census);

        assert_eq!(first, LONG.len());
        assert_eq!(
            second, 0,
            "a clone shares the buffer and must not be charged"
        );
    }

    #[test]
    fn short_text_stored_inline_costs_no_heap_at_all() {
        // Small strings live in the value's own bytes, so there is no
        // allocation to charge and nothing for two clones to share.
        let mut census = Census::new();
        let short = gpui::SharedString::from("main.rs".to_string());
        assert_eq!(short.heap_size(&mut census), 0);
        assert_eq!(short.clone().heap_size(&mut census), 0);
    }

    #[test]
    fn distinct_shared_strings_are_charged_separately() {
        let mut census = Census::new();
        let first = gpui::SharedString::from(format!("{LONG} one"));
        let second = gpui::SharedString::from(format!("{LONG} two"));

        assert_eq!(first.heap_size(&mut census), LONG.len() + 4);
        assert_eq!(second.heap_size(&mut census), LONG.len() + 4);
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
