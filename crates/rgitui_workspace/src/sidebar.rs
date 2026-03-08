use std::collections::BTreeMap;
use std::collections::HashSet;

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
    MergeBranch(String),
    TagSelected(String),
    StashSelected(usize),
    FileSelected { path: String, staged: bool },
    StageFile(String),
    UnstageFile(String),
    StageAll,
    UnstageAll,
    DiscardFile(String),
    OpenRepo,
}

/// Sidebar sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    /// Tracks which directory groups are collapsed in the file change sections.
    /// Key is "staged:<dir>" or "unstaged:<dir>".
    collapsed_dirs: HashSet<String>,
    /// Current repository name displayed in the sidebar header.
    repo_name: String,
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
            collapsed_dirs: HashSet::new(),
            repo_name: String::new(),
        }
    }

    pub fn set_repo_name(&mut self, name: String, cx: &mut Context<Self>) {
        self.repo_name = name;
        cx.notify();
    }

    pub fn update_branches(&mut self, mut branches: Vec<BranchInfo>, cx: &mut Context<Self>) {
        // Sort branches alphabetically, with HEAD branch first among locals
        branches.sort_by(|a, b| {
            if a.is_remote != b.is_remote {
                return a.is_remote.cmp(&b.is_remote);
            }
            // HEAD branch sorts first within its group
            if a.is_head != b.is_head {
                return b.is_head.cmp(&a.is_head);
            }
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        });
        self.branches = branches;
        cx.notify();
    }

    pub fn update_tags(&mut self, mut tags: Vec<TagInfo>, cx: &mut Context<Self>) {
        tags.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
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
        // Note: we intentionally do NOT reset collapsed_dirs here
        // so that collapsed directory groups persist across refreshes.
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

    fn is_dir_collapsed(&self, prefix: &str, dir: &str) -> bool {
        self.collapsed_dirs.contains(&format!("{}:{}", prefix, dir))
    }

    fn toggle_dir(&mut self, prefix: &str, dir: &str, cx: &mut Context<Self>) {
        let key = format!("{}:{}", prefix, dir);
        if self.collapsed_dirs.contains(&key) {
            self.collapsed_dirs.remove(&key);
        } else {
            self.collapsed_dirs.insert(key);
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

    /// Map a file change kind to an appropriate icon.
    fn file_change_icon(kind: FileChangeKind) -> IconName {
        match kind {
            FileChangeKind::Added => IconName::FileAdded,
            FileChangeKind::Modified => IconName::FileModified,
            FileChangeKind::Deleted => IconName::FileDeleted,
            FileChangeKind::Renamed => IconName::FileRenamed,
            FileChangeKind::Copied => IconName::FileRenamed,
            FileChangeKind::TypeChange => IconName::FileModified,
            FileChangeKind::Untracked => IconName::FileAdded,
            FileChangeKind::Conflicted => IconName::FileConflict,
        }
    }

    /// Group files by their parent directory. Returns a BTreeMap so directories
    /// are sorted alphabetically. Files in the root directory use "" as key.
    fn group_files_by_dir(files: &[FileStatus]) -> BTreeMap<String, Vec<&FileStatus>> {
        let mut groups: BTreeMap<String, Vec<&FileStatus>> = BTreeMap::new();
        for file in files {
            let dir = file
                .path
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_string();
            groups.entry(dir).or_default().push(file);
        }
        // Sort files within each group alphabetically
        for files in groups.values_mut() {
            files.sort_by(|a, b| {
                let a_name = a.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let b_name = b.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                a_name.to_lowercase().cmp(&b_name.to_lowercase())
            });
        }
        groups
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

        // -- Sidebar Header: repo name + open repo button --
        {
            let repo_label: SharedString = if self.repo_name.is_empty() {
                "No Repository".into()
            } else {
                self.repo_name.clone().into()
            };

            panel = panel.child(
                div()
                    .id("sidebar-header")
                    .h_flex()
                    .w_full()
                    .h(px(32.))
                    .px(px(10.))
                    .gap(px(6.))
                    .items_center()
                    .bg(colors.surface_background)
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .child(
                        rgitui_ui::Icon::new(IconName::Folder)
                            .size(rgitui_ui::IconSize::Small)
                            .color(Color::Accent),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                Label::new(repo_label)
                                    .size(LabelSize::Small)
                                    .weight(gpui::FontWeight::SEMIBOLD)
                                    .truncate(),
                            ),
                    )
                    .child(
                        IconButton::new("sidebar-open-repo", IconName::Plus)
                            .size(ButtonSize::Compact)
                            .color(Color::Muted)
                            .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                cx.emit(SidebarEvent::OpenRepo);
                            })),
                    ),
            );
        }

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
                .h(px(28.))
                .px(px(10.))
                .gap(px(6.))
                .items_center()
                .bg(colors.surface_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .hover(|s| s.bg(colors.ghost_element_hover))
                .cursor_pointer()
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
                        .weight(gpui::FontWeight::BOLD)
                        .color(Color::Muted),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .h_flex()
                        .h(px(16.))
                        .min_w(px(20.))
                        .px(px(6.))
                        .rounded(px(8.))
                        .bg(colors.ghost_element_hover)
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(SharedString::from(format!("{}", local_branches.len())))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                ),
        );

        if local_expanded {
            for (i, branch) in local_branches.iter().enumerate() {
                let name: SharedString = branch.name.clone().into();
                let branch_name = branch.name.clone();
                let branch_name_select = branch.name.clone();
                let branch_name_merge = branch.name.clone();
                let is_head = branch.is_head;

                let mut item = div()
                    .id(ElementId::NamedInteger("local-branch".into(), i as u64))
                    .h_flex()
                    .w_full()
                    .h(px(28.))
                    .px_2()
                    .pl(px(20.))
                    .gap_1()
                    .items_center()
                    .overflow_hidden()
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .cursor_pointer()
                    .on_click(cx.listener(move |_this, event: &ClickEvent, _, cx| {
                        if event.click_count() >= 2 {
                            // Double-click: checkout this branch
                            cx.emit(SidebarEvent::BranchCheckout(branch_name.clone()));
                        } else {
                            // Single-click: select
                            cx.emit(SidebarEvent::BranchSelected(branch_name_select.clone()));
                        }
                    }));

                if is_head {
                    // Active branch: filled dot icon + accent color + bold + highlight bg
                    item = item
                        .bg(colors.ghost_element_selected)
                        .child(
                            rgitui_ui::Icon::new(IconName::Dot)
                                .size(rgitui_ui::IconSize::XSmall)
                                .color(Color::Accent),
                        )
                        .child(
                            rgitui_ui::Icon::new(IconName::GitBranch)
                                .size(rgitui_ui::IconSize::XSmall)
                                .color(Color::Accent),
                        )
                        .child(
                            Label::new(name)
                                .size(LabelSize::XSmall)
                                .color(Color::Accent)
                                .weight(gpui::FontWeight::BOLD)
                                .truncate(),
                        );
                } else {
                    // Non-HEAD branch: empty circle outline icon + muted color
                    item = item
                        .child(
                            rgitui_ui::Icon::new(IconName::DotOutline)
                                .size(rgitui_ui::IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            rgitui_ui::Icon::new(IconName::GitBranch)
                                .size(rgitui_ui::IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new(name)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .truncate(),
                        );
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

                // Non-HEAD branches get "Merge" and "Delete" buttons
                if !is_head {
                    let branch_name_delete = branch.name.clone();
                    item = item.child(
                        div().ml_auto().h_flex().gap(px(2.)).child(
                            IconButton::new(
                                ElementId::NamedInteger("merge-branch".into(), i as u64),
                                IconName::GitMerge,
                            )
                            .size(ButtonSize::Compact)
                            .color(Color::Muted)
                            .on_click(cx.listener(
                                move |_this, _: &ClickEvent, _, cx| {
                                    cx.emit(SidebarEvent::MergeBranch(
                                        branch_name_merge.clone(),
                                    ));
                                },
                            )),
                        ).child(
                            IconButton::new(
                                ElementId::NamedInteger("delete-branch".into(), i as u64),
                                IconName::Trash,
                            )
                            .size(ButtonSize::Compact)
                            .color(Color::Deleted)
                            .on_click(cx.listener(
                                move |_this, _: &ClickEvent, _, cx| {
                                    cx.emit(SidebarEvent::BranchDelete(
                                        branch_name_delete.clone(),
                                    ));
                                },
                            )),
                        ),
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
                .h(px(28.))
                .px(px(10.))
                .gap(px(6.))
                .items_center()
                .bg(colors.surface_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .hover(|s| s.bg(colors.ghost_element_hover))
                .cursor_pointer()
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
                        .weight(gpui::FontWeight::BOLD)
                        .color(Color::Muted),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .h_flex()
                        .h(px(16.))
                        .min_w(px(20.))
                        .px(px(6.))
                        .rounded(px(8.))
                        .bg(colors.ghost_element_hover)
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(SharedString::from(format!("{}", remote_branches.len())))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
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
                        .h(px(28.))
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
                            rgitui_ui::Icon::new(IconName::GitBranch)
                                .size(rgitui_ui::IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new(name)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .truncate(),
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
                .h(px(28.))
                .px(px(10.))
                .gap(px(6.))
                .items_center()
                .bg(colors.surface_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .hover(|s| s.bg(colors.ghost_element_hover))
                .cursor_pointer()
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
                        .weight(gpui::FontWeight::BOLD)
                        .color(Color::Muted),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .h_flex()
                        .h(px(16.))
                        .min_w(px(20.))
                        .px(px(6.))
                        .rounded(px(8.))
                        .bg(colors.ghost_element_hover)
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(SharedString::from(format!("{}", self.tags.len())))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
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
                        .h(px(28.))
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
                            rgitui_ui::Icon::new(IconName::Tag)
                                .size(rgitui_ui::IconSize::XSmall)
                                .color(Color::Warning),
                        )
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
                .h(px(28.))
                .px(px(10.))
                .gap(px(6.))
                .items_center()
                .bg(colors.surface_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .hover(|s| s.bg(colors.ghost_element_hover))
                .cursor_pointer()
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
                        .weight(gpui::FontWeight::BOLD)
                        .color(Color::Muted),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .h_flex()
                        .h(px(16.))
                        .min_w(px(20.))
                        .px(px(6.))
                        .rounded(px(8.))
                        .bg(colors.ghost_element_hover)
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(SharedString::from(format!("{}", self.stashes.len())))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
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
                        .h(px(28.))
                        .px_2()
                        .pl(px(20.))
                        .gap_1()
                        .items_center()
                        .overflow_hidden()
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(SidebarEvent::StashSelected(stash_index));
                        }))
                        .child(
                            rgitui_ui::Icon::new(IconName::Stash)
                                .size(rgitui_ui::IconSize::XSmall)
                                .color(Color::Muted),
                        )
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
                .h(px(28.))
                .px(px(10.))
                .gap(px(6.))
                .items_center()
                .bg(colors.surface_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .hover(|s| s.bg(colors.ghost_element_hover))
                .cursor_pointer()
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
                        .weight(gpui::FontWeight::BOLD)
                        .color(Color::Muted),
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
                    div()
                        .h_flex()
                        .h(px(16.))
                        .min_w(px(20.))
                        .px(px(6.))
                        .rounded(px(8.))
                        .bg(if staged_count > 0 {
                            colors.ghost_element_selected
                        } else {
                            colors.ghost_element_hover
                        })
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(SharedString::from(format!("{}", staged_count)))
                                .size(LabelSize::XSmall)
                                .color(if staged_count > 0 {
                                    Color::Added
                                } else {
                                    Color::Muted
                                }),
                        ),
                ),
        );

        if staged_expanded {
            let staged_files = self.staged.clone();
            let grouped = Self::group_files_by_dir(&staged_files);
            let mut file_idx: usize = 0;

            for (dir, files) in &grouped {
                // If there's a non-empty directory, render a collapsible header
                if !dir.is_empty() {
                    let dir_collapsed = self.is_dir_collapsed("staged", dir);
                    let dir_icon = if dir_collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    };
                    let dir_display: SharedString = format!("{}/", dir).into();
                    let dir_clone = dir.clone();

                    panel = panel.child(
                        div()
                            .id(SharedString::from(format!("staged-dir-{}", dir)))
                            .h_flex()
                            .w_full()
                            .h(px(28.))
                            .px_2()
                            .pl(px(16.))
                            .gap_1()
                            .items_center()
                            .overflow_hidden()
                            .hover(|s| s.bg(colors.ghost_element_hover))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.toggle_dir("staged", &dir_clone, cx);
                            }))
                            .child(
                                rgitui_ui::Icon::new(dir_icon)
                                    .size(rgitui_ui::IconSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                rgitui_ui::Icon::new(IconName::Folder)
                                    .size(rgitui_ui::IconSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new(dir_display)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                                    .truncate(),
                            ),
                    );

                    if dir_collapsed {
                        file_idx += files.len();
                        continue;
                    }
                }

                let indent = if dir.is_empty() { px(20.) } else { px(32.) };

                for file in files {
                    let i = file_idx;
                    file_idx += 1;

                    let file_name: SharedString = file
                        .path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string()
                        .into();
                    let kind_icon = Self::file_change_icon(file.kind);
                    let color = Self::file_change_color(file.kind);
                    let additions = file.additions;
                    let deletions = file.deletions;
                    let file_path = file.path.display().to_string();
                    let file_path_select = file_path.clone();
                    let file_path_unstage = file_path.clone();
                    let is_selected = self
                        .selected_file
                        .as_ref()
                        .map_or(false, |(p, s)| p == &file_path && *s);

                    let has_stats = additions > 0 || deletions > 0;

                    panel = panel.child(
                        div()
                            .id(ElementId::NamedInteger("staged-file".into(), i as u64))
                            .h_flex()
                            .w_full()
                            .h(px(28.))
                            .px_2()
                            .pl(indent)
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
                                rgitui_ui::Icon::new(kind_icon)
                                    .size(rgitui_ui::IconSize::XSmall)
                                    .color(color),
                            )
                            .child(
                                Label::new(file_name)
                                    .size(LabelSize::XSmall)
                                    .color(color)
                                    .truncate(),
                            )
                            .child(div().flex_1())
                            .when(has_stats, |el| {
                                el.child(
                                    div()
                                        .h_flex()
                                        .gap(px(4.))
                                        .flex_shrink_0()
                                        .child(
                                            Label::new(SharedString::from(format!("+{}", additions)))
                                                .size(LabelSize::XSmall)
                                                .color(Color::Added),
                                        )
                                        .child(
                                            Label::new(SharedString::from(format!("-{}", deletions)))
                                                .size(LabelSize::XSmall)
                                                .color(Color::Deleted),
                                        ),
                                )
                            })
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
                                        .on_click(cx.listener(
                                            move |_this, _: &ClickEvent, _, cx| {
                                                cx.emit(SidebarEvent::UnstageFile(
                                                    file_path_unstage.clone(),
                                                ));
                                            },
                                        )),
                                    ),
                            ),
                    );
                }
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
                .h(px(28.))
                .px(px(10.))
                .gap(px(6.))
                .items_center()
                .bg(colors.surface_background)
                .border_b_1()
                .border_color(colors.border_variant)
                .hover(|s| s.bg(colors.ghost_element_hover))
                .cursor_pointer()
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
                        .weight(gpui::FontWeight::BOLD)
                        .color(Color::Muted),
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
                    div()
                        .h_flex()
                        .h(px(16.))
                        .min_w(px(20.))
                        .px(px(6.))
                        .rounded(px(8.))
                        .bg(if unstaged_count > 0 {
                            colors.ghost_element_selected
                        } else {
                            colors.ghost_element_hover
                        })
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(SharedString::from(format!("{}", unstaged_count)))
                                .size(LabelSize::XSmall)
                                .color(if unstaged_count > 0 {
                                    Color::Modified
                                } else {
                                    Color::Muted
                                }),
                        ),
                ),
        );

        if unstaged_expanded {
            let unstaged_files = self.unstaged.clone();
            let grouped = Self::group_files_by_dir(&unstaged_files);
            let mut file_idx: usize = 0;

            for (dir, files) in &grouped {
                // If there's a non-empty directory, render a collapsible header
                if !dir.is_empty() {
                    let dir_collapsed = self.is_dir_collapsed("unstaged", dir);
                    let dir_icon = if dir_collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    };
                    let dir_display: SharedString = format!("{}/", dir).into();
                    let dir_clone = dir.clone();

                    panel = panel.child(
                        div()
                            .id(SharedString::from(format!("unstaged-dir-{}", dir)))
                            .h_flex()
                            .w_full()
                            .h(px(28.))
                            .px_2()
                            .pl(px(16.))
                            .gap_1()
                            .items_center()
                            .overflow_hidden()
                            .hover(|s| s.bg(colors.ghost_element_hover))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.toggle_dir("unstaged", &dir_clone, cx);
                            }))
                            .child(
                                rgitui_ui::Icon::new(dir_icon)
                                    .size(rgitui_ui::IconSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                rgitui_ui::Icon::new(IconName::Folder)
                                    .size(rgitui_ui::IconSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new(dir_display)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                                    .truncate(),
                            ),
                    );

                    if dir_collapsed {
                        file_idx += files.len();
                        continue;
                    }
                }

                let indent = if dir.is_empty() { px(20.) } else { px(32.) };

                for file in files {
                    let i = file_idx;
                    file_idx += 1;

                    let file_name: SharedString = file
                        .path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string()
                        .into();
                    let kind_icon = Self::file_change_icon(file.kind);
                    let color = Self::file_change_color(file.kind);
                    let additions = file.additions;
                    let deletions = file.deletions;
                    let file_path = file.path.display().to_string();
                    let file_path_select = file_path.clone();
                    let file_path_stage = file_path.clone();
                    let file_path_discard = file_path.clone();
                    let is_selected = self
                        .selected_file
                        .as_ref()
                        .map_or(false, |(p, s)| p == &file_path && !*s);

                    let has_stats = additions > 0 || deletions > 0;

                    panel = panel.child(
                        div()
                            .id(ElementId::NamedInteger("unstaged-file".into(), i as u64))
                            .h_flex()
                            .w_full()
                            .h(px(28.))
                            .px_2()
                            .pl(indent)
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
                                rgitui_ui::Icon::new(kind_icon)
                                    .size(rgitui_ui::IconSize::XSmall)
                                    .color(color),
                            )
                            .child(
                                Label::new(file_name)
                                    .size(LabelSize::XSmall)
                                    .color(color)
                                    .truncate(),
                            )
                            .child(div().flex_1())
                            .when(has_stats, |el| {
                                el.child(
                                    div()
                                        .h_flex()
                                        .gap(px(4.))
                                        .flex_shrink_0()
                                        .child(
                                            Label::new(SharedString::from(format!("+{}", additions)))
                                                .size(LabelSize::XSmall)
                                                .color(Color::Added),
                                        )
                                        .child(
                                            Label::new(SharedString::from(format!("-{}", deletions)))
                                                .size(LabelSize::XSmall)
                                                .color(Color::Deleted),
                                        ),
                                )
                            })
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
                                        .on_click(cx.listener(
                                            move |_this, _: &ClickEvent, _, cx| {
                                                cx.emit(SidebarEvent::StageFile(
                                                    file_path_stage.clone(),
                                                ));
                                            },
                                        )),
                                    ),
                            )
                            .child(
                                IconButton::new(
                                    ElementId::NamedInteger("discard-file".into(), i as u64),
                                    IconName::X,
                                )
                                .size(ButtonSize::Compact)
                                .color(Color::Deleted)
                                .on_click(cx.listener(
                                    move |_this, _: &ClickEvent, _, cx| {
                                        cx.emit(SidebarEvent::DiscardFile(
                                            file_path_discard.clone(),
                                        ));
                                    },
                                )),
                            ),
                    );
                }
            }
        }

        panel
    }
}
