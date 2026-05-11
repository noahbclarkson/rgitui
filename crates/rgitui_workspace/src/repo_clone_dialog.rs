use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    div, px, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle, FontWeight,
    KeyDownEvent, Render, SharedString, Window,
};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{
    Button, ButtonStyle, Icon, IconName, IconSize, Label, LabelSize, TextInput, TextInputEvent,
};

#[derive(Debug, Clone)]
pub enum RepoCloneEvent {
    CloneRepo { url: String, path: PathBuf },
    Dismissed,
}

// ... etc
