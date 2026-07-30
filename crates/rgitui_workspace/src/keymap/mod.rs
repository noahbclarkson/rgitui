//! User-definable keybindings.
//!
//! * [`macros`] defines `commands!`, the single source of truth.
//! * [`registry`] declares every command with it, producing [`CommandId`], the
//!   gpui action structs and [`ALL_COMMANDS`].

#[macro_use]
pub(crate) mod macros;

pub mod registry;

pub use registry::{actions, attach_actions, CommandId, CommandMeta, ALL_COMMANDS};
