//! Test fixtures shared across the rgitui workspace.
//!
//! Each fixture owns a resource that tests otherwise have to remember to keep
//! alive or tear down in the right order: [`ViewTest`] owns a headless GPUI app
//! and its window, [`TempRepo`] owns a temporary git repository.

mod temp_repo;
mod view_test;

pub use temp_repo::TempRepo;
pub use view_test::ViewTest;
