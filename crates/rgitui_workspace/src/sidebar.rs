use gpui::prelude::*;
use gpui::{div, px, ClickEvent, Context, ElementId, EventEmitter, Render, SharedString, Window};
use rgitui_git::{BranchInfo, FileChangeKind, FileStatus, RemoteInfo, StashEntry, TagInfo};
use rgitui_theme::{ActiveTheme, Color, StyledExt};
use rgitui_ui::{Button, ButtonSize, ButtonStyle, IconButton, IconName, Label, LabelSize};

/// Events from the sidebar.
#[derive(Debug, Clone)]
pub enum SidebarEvent {
    BranchSelected(String),
    BranchCheckout(String),
    BranchCreate,
    BranchDelete(String),
    TagSelected(String),
    StashSelected(usize),
    FileSelected { path: String, staged: bool },
    StageFile(String),
    UnstageFile(String),
    StageAll,
    UnstageAll,
    DiscardFile(String),
}

/// Sidebar sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarSection {
    LocalBranches,
    RemoteBranches,
    Tags,
    Stashes,
    StagedChanges,
    UnstagedChanges,
}

/// The left sidebar panel with branches, tags, stashes, and working tree status.
pub struct Sidebar {
    expanded_sections: Vec<SidebarSection>,
    branches: Vec<BranchInfo>,
    tags: Vec<TagInfo>,
    remotes: Vec<RemoteInfo>,
    stashes: Vec<StashEntry>,
    staged: Vec<FileStatus>,
    unstaged: Vec<FileStatus>,
    selected_file: Option<(String, bool)>, // (path, is_staged)
}

impl EventEmitter<SidebarEvent> for Sidebar {}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            expanded_sections: vec![
                SidebarSection::LocalBranches,
                SidebarSection::StagedChanges,
                SidebarSection::UnstagedChanges,
            ],
            branches: Vec::new(),
            tags: Vec::new(),
            remotes: Vec::new(),
            stashes: Vec::new(),
            staged: Vec::new(),
            unstaged: Vec::new(),
            selected_file: None,
        }
    }

    pub fn update_branches(&mut self, branches: Vec<BranchInfo>, cx: &mut Context<Self>) {
        self.branches = branches;
        cx.notify();
    }

    pub fn update_tags(&mut self, tags: Vec<TagInfo>, cx: &mut Context<Self>) {
        self.tags = tags;
        cx.notify();
    }

    pub fn update_remotes(&mut self, remotes: Vec<RemoteInfo>, cx: &mut Context<Self>) {
        self.remotes = remotes;
        cx.notify();
    }

    pub fn update_stashes(&mut self, stashes: Vec<StashEntry>, cx: &mut Context<Self>) {
        self.stashes = stashes;
        cx.notify();
    }

    pub fn update_status(
        &mut self,
        staged: Vec<FileStatus>,
        unstaged: Vec<FileStatus>,
        cx: &mut Context<Self>,
    ) {
        self.staged = staged;
        self.unstaged = unstaged;
        cx.notify();
    }

    fn is_expanded(&self, section: SidebarSection) -> bool {
        self.expanded_sections.contains(&section)
    }

    fn toggle_section(&mut self, section: SidebarSection, cx: &mut Context<Self>) {
        if let Some(pos) = self.expanded_sections.iter().position(|s| *s == section) {
            self.expanded_sections.remove(pos);
        } else {
            self.expanded_sections.push(section);
        }
        cx.notify();
    }

    fn file_change_color(kind: FileChangeKind) -> Color {
        match kind {
            FileChangeKind::Added => Color::Added,
            FileChangeKind::Modified => Color::Modified,
            FileChangeKind::Deleted => Color::Deleted,
            FileChangeKind::Renamed => Color::Renamed,
            FileChangeKind::Copied => Color::Info,
            FileChangeKind::TypeChange => Color::Warning,
            FileChangeKind::Untracked => Color::Untracked,
            FileChangeKind::Conflicted => Color::Conflict,
        }
    }
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();

        let mut panel = div()
            .id("sidebar-panel")
            .v_flex()
            .w_full()
            .h_full()
            .bg(colors.panel_background)
            .border_r_1()
            .border_color(colors.border_variant)
            .overflow_y_scroll();

        // -- Local Branches --
        let local_branches: Vec<BranchInfo> = self
            .branches
            .iter()
            .filter(|b| !b.is_remote)
            .cloned()
            .collect();

        let local_expanded = self.is_expanded(SidebarSection::LocalBranches);
        let icon = if local_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        panel = panel.child(
            div()
                .id("section-local-branches")
                .h_flex()
                .w_full()
                .h(px(26.))
                .px_2()
                .gap_1()
                .items_center()
                .bg(colors.surface_background)
                .hover(|s| s.bg(colors.ghost_element_hover))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_section(SidebarSection::LocalBranches, cx);
                }))
                .child(
                    rgitui_ui::Icon::new(icon)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    Label::new("LOCAL BRANCHES")
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD),
                )
                .child(div().flex_1())
                .child(
                    Label::new(SharedString::from(format!("{}", local_branches.len())))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
        );

        if local_expanded {
            for (i, branch) in local_branches.iter().enumerate() {
                let name: SharedString = branch.name.clone().into();
                let branch_name = branch.name.clone();
                let mut item = div()
                    .id(ElementId::NamedInteger("local-branch".into(), i as u64))
                    .h_flex()
                    .w_full()
                    .h(px(24.))
                    .px_2()
                    .pl(px(20.))
                    .gap_1()
                    .items_center()
                    .overflow_hidden()
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                        cx.emit(SidebarEvent::BranchCheckout(branch_name.clone()));
                    }));

                if branch.is_head {
                    item = item.child(
                        Label::new(name)
                            .size(LabelSize::XSmall)
                            .color(Color::Accent)
                            .weight(gpui::FontWeight::BOLD),
                    );
                } else {
                    item = item.child(Label::new(name).size(LabelSize::XSmall));
                }

                if branch.ahead > 0 || branch.behind > 0 {
                    let sync: SharedString =
                        format!("↑{} ↓{}", branch.ahead, branch.behind).into();
                    item = item.child(
                        Label::new(sync)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    );
                }

                panel = panel.child(item);
            }
        }

        // -- Remote Branches --
        let remote_branches: Vec<BranchInfo> = self
            .branches
            .iter()
            .filter(|b| b.is_remote)
            .cloned()
            .collect();

        let remote_expanded = self.is_expanded(SidebarSection::RemoteBranches);
        let icon = if remote_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        panel = panel.child(
            div()
                .id("section-remote-branches")
                .h_flex()
                .w_full()
                .h(px(26.))
                .px_2()
                .gap_1()
                .items_center()
                .bg(colors.surface_background)
                .hover(|s| s.bg(colors.ghost_element_hover))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_section(SidebarSection::RemoteBranches, cx);
                }))
                .child(
                    rgitui_ui::Icon::new(icon)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    Label::new("REMOTE BRANCHES")
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD),
                )
                .child(div().flex_1())
                .child(
                    Label::new(SharedString::from(format!("{}", remote_branches.len())))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
        );

        if remote_expanded {
            for (i, branch) in remote_branches.iter().enumerate() {
                let name: SharedString = branch.name.clone().into();
                let remote_branch_name = branch.name.clone();
                panel = panel.child(
                    div()
                        .id(ElementId::NamedInteger("remote-branch".into(), i as u64))
                        .h_flex()
                        .w_full()
                        .h(px(24.))
                        .px_2()
                        .pl(px(20.))
                        .gap_1()
                        .items_center()
                        .overflow_hidden()
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(SidebarEvent::BranchSelected(remote_branch_name.clone()));
                        }))
                        .child(
                            Label::new(name)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                );
            }
        }

        // -- Tags --
        let tags_expanded = self.is_expanded(SidebarSection::Tags);
        let icon = if tags_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        panel = panel.child(
            div()
                .id("section-tags")
                .h_flex()
                .w_full()
                .h(px(26.))
                .px_2()
                .gap_1()
                .items_center()
                .bg(colors.surface_background)
                .hover(|s| s.bg(colors.ghost_element_hover))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_section(SidebarSection::Tags, cx);
                }))
                .child(
                    rgitui_ui::Icon::new(icon)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    Label::new("TAGS")
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD),
                )
                .child(div().flex_1())
                .child(
                    Label::new(SharedString::from(format!("{}", self.tags.len())))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
        );

        if tags_expanded {
            for (i, tag) in self.tags.iter().enumerate() {
                let name: SharedString = tag.name.clone().into();
                let tag_name = tag.name.clone();
                panel = panel.child(
                    div()
                        .id(ElementId::NamedInteger("tag-item".into(), i as u64))
                        .h_flex()
                        .w_full()
                        .h(px(24.))
                        .px_2()
                        .pl(px(20.))
                        .gap_1()
                        .items_center()
                        .overflow_hidden()
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(SidebarEvent::TagSelected(tag_name.clone()));
                        }))
                        .child(
                            Label::new(name)
                                .size(LabelSize::XSmall)
                                .color(Color::Warning),
                        ),
                );
            }
        }

        // -- Stashes --
        let stashes_expanded = self.is_expanded(SidebarSection::Stashes);
        let icon = if stashes_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        panel = panel.child(
            div()
                .id("section-stashes")
                .h_flex()
                .w_full()
                .h(px(26.))
                .px_2()
                .gap_1()
                .items_center()
                .bg(colors.surface_background)
                .hover(|s| s.bg(colors.ghost_element_hover))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_section(SidebarSection::Stashes, cx);
                }))
                .child(
                    rgitui_ui::Icon::new(icon)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    Label::new("STASHES")
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD),
                )
                .child(div().flex_1())
                .child(
                    Label::new(SharedString::from(format!("{}", self.stashes.len())))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
        );

        if stashes_expanded {
            for (i, stash) in self.stashes.iter().enumerate() {
                let msg: SharedString = stash.message.clone().into();
                let stash_index = stash.index;
                panel = panel.child(
                    div()
                        .id(ElementId::NamedInteger("stash-item".into(), i as u64))
                        .h_flex()
                        .w_full()
                        .h(px(24.))
                        .px_2()
                        .pl(px(20.))
                        .gap_1()
                        .items_center()
                        .overflow_hidden()
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(SidebarEvent::StashSelected(stash_index));
                        }))
                        .child(Label::new(msg).size(LabelSize::XSmall).truncate()),
                );
            }
        }

        // Separator
        panel = panel.child(
            div()
                .w_full()
                .h(px(1.))
                .my_1()
                .bg(colors.border_variant),
        );

        // -- Staged Changes --
        let staged_expanded = self.is_expanded(SidebarSection::StagedChanges);
        let staged_icon = if staged_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };
        let staged_count = self.staged.len();
        let has_staged = staged_count > 0;

        panel = panel.child(
            div()
                .id("section-staged")
                .h_flex()
                .w_full()
                .h(px(26.))
                .px_2()
                .gap_1()
                .items_center()
                .bg(colors.surface_background)
                .hover(|s| s.bg(colors.ghost_element_hover))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_section(SidebarSection::StagedChanges, cx);
                }))
                .child(
                    rgitui_ui::Icon::new(staged_icon)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    Label::new("STAGED CHANGES")
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD),
                )
                .child(div().flex_1())
                .when(has_staged, |el| {
                    el.child(
                        div()
                            .id("unstage-all-btn")
                            .child(
                                Button::new("unstage-all", "−")
                                    .size(ButtonSize::Compact)
                                    .style(ButtonStyle::Subtle)
                                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                        cx.emit(SidebarEvent::UnstageAll);
                                    })),
                            ),
                    )
                })
                .child(
                    Label::new(SharedString::from(format!("{}", staged_count)))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
        );

        if staged_expanded {
            let staged_files = self.staged.clone();
            for (i, file) in staged_files.iter().enumerate() {
                let file_name: SharedString = file
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string()
                    .into();
                let kind_str: SharedString = file.kind.short_code().to_string().into();
                let color = Self::file_change_color(file.kind);
                let file_path = file.path.display().to_string();
                let file_path_select = file_path.clone();
                let file_path_unstage = file_path.clone();
                let is_selected = self
                    .selected_file
                    .as_ref()
                    .map_or(false, |(p, s)| p == &file_path && *s);

                panel = panel.child(
                    div()
                        .id(ElementId::NamedInteger("staged-file".into(), i as u64))
                        .h_flex()
                        .w_full()
                        .h(px(24.))
                        .px_2()
                        .pl(px(20.))
                        .gap_1()
                        .items_center()
                        .overflow_hidden()
                        .when(is_selected, |el| el.bg(colors.ghost_element_selected))
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.selected_file = Some((file_path_select.clone(), true));
                            cx.emit(SidebarEvent::FileSelected {
                                path: file_path_select.clone(),
                                staged: true,
                            });
                            cx.notify();
                        }))
                        .child(
                            Label::new(kind_str)
                                .size(LabelSize::XSmall)
                                .color(color),
                        )
                        .child(
                            Label::new(file_name)
                                .size(LabelSize::XSmall)
                                .truncate(),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .id(ElementId::NamedInteger("unstage-btn".into(), i as u64))
                                .child(
                                    Button::new(
                                        SharedString::from(format!("unstage-{}", i)),
                                        "−",
                                    )
                                    .size(ButtonSize::Compact)
                                    .style(ButtonStyle::Subtle)
                                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                                        cx.emit(SidebarEvent::UnstageFile(
                                            file_path_unstage.clone(),
                                        ));
                                    })),
                                ),
                        ),
                );
            }
        }

        // -- Unstaged Changes --
        let unstaged_expanded = self.is_expanded(SidebarSection::UnstagedChanges);
        let unstaged_icon = if unstaged_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };
        let unstaged_count = self.unstaged.len();
        let has_unstaged = unstaged_count > 0;

        panel = panel.child(
            div()
                .id("section-unstaged")
                .h_flex()
                .w_full()
                .h(px(26.))
                .px_2()
                .gap_1()
                .items_center()
                .bg(colors.surface_background)
                .hover(|s| s.bg(colors.ghost_element_hover))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_section(SidebarSection::UnstagedChanges, cx);
                }))
                .child(
                    rgitui_ui::Icon::new(unstaged_icon)
                        .size(rgitui_ui::IconSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    Label::new("UNSTAGED CHANGES")
                        .size(LabelSize::XSmall)
                        .weight(gpui::FontWeight::SEMIBOLD),
                )
                .child(div().flex_1())
                .when(has_unstaged, |el| {
                    el.child(
                        div()
                            .id("stage-all-btn")
                            .child(
                                Button::new("stage-all", "+")
                                    .size(ButtonSize::Compact)
                                    .style(ButtonStyle::Subtle)
                                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                        cx.emit(SidebarEvent::StageAll);
                                    })),
                            ),
                    )
                })
                .child(
                    Label::new(SharedString::from(format!("{}", unstaged_count)))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
        );

        if unstaged_expanded {
            let unstaged_files = self.unstaged.clone();
            for (i, file) in unstaged_files.iter().enumerate() {
                let file_name: SharedString = file
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string()
                    .into();
                let kind_str: SharedString = file.kind.short_code().to_string().into();
                let color = Self::file_change_color(file.kind);
                let file_path = file.path.display().to_string();
                let file_path_select = file_path.clone();
                let file_path_stage = file_path.clone();
                let file_path_discard = file_path.clone();
                let is_selected = self
                    .selected_file
                    .as_ref()
                    .map_or(false, |(p, s)| p == &file_path && !*s);

                panel = panel.child(
                    div()
                        .id(ElementId::NamedInteger("unstaged-file".into(), i as u64))
                        .h_flex()
                        .w_full()
                        .h(px(24.))
                        .px_2()
                        .pl(px(20.))
                        .gap_1()
                        .items_center()
                        .overflow_hidden()
                        .when(is_selected, |el| el.bg(colors.ghost_element_selected))
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.selected_file = Some((file_path_select.clone(), false));
                            cx.emit(SidebarEvent::FileSelected {
                                path: file_path_select.clone(),
                                staged: false,
                            });
                            cx.notify();
                        }))
                        .child(
                            Label::new(kind_str)
                                .size(LabelSize::XSmall)
                                .color(color),
                        )
                        .child(
                            Label::new(file_name)
                                .size(LabelSize::XSmall)
                                .truncate(),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .id(ElementId::NamedInteger("stage-btn".into(), i as u64))
                                .child(
                                    Button::new(
                                        SharedString::from(format!("stage-{}", i)),
                                        "+",
                                    )
                                    .size(ButtonSize::Compact)
                                    .style(ButtonStyle::Subtle)
                                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                                        cx.emit(SidebarEvent::StageFile(
                                            file_path_stage.clone(),
                                        ));
                                    })),
                                ),
                        )
                        .child(
                            IconButton::new(
                                ElementId::NamedInteger("discard-file".into(), i as u64),
                                IconName::X,
                            )
                            .size(ButtonSize::Compact)
                            .color(Color::Deleted)
                            .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                                cx.emit(SidebarEvent::DiscardFile(file_path_discard.clone()));
                            })),
                        ),
                );
            }
        }

        panel
    }
}
