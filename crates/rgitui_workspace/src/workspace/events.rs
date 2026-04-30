use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use gpui::{Context, Entity, SharedString};
use rgitui_ai::{AiEvent, AiGenerator};
use rgitui_diff::{DiffViewer, DiffViewerEvent};
use rgitui_git::{
    CommitInfo, GitOperationKind, GitOperationState, GitProject, GitProjectEvent,
    RebaseEntryAction, RebasePlanEntry, Signature,
};
use rgitui_graph::{GraphView, GraphViewEvent, WorktreeGraphInfo};

use crate::{
    cache::LruCache, BisectView, BisectViewEvent, BlameView, BlameViewEvent, BranchDialog,
    BranchDialogEvent, CommandPalette, CommandPaletteEvent, CommitPanel, CommitPanelEvent,
    ConfirmAction, ConfirmDialog, ConfirmDialogEvent, CreatePrDialog, CreatePrDialogEvent,
    DetailPanel, DetailPanelEvent, FileHistoryView, FileHistoryViewEvent, GlobalSearchView,
    GlobalSearchViewEvent, InteractiveRebase, InteractiveRebaseEvent, ReflogView, ReflogViewEvent,
    RenameDialog, RenameDialogEvent, RepoOpener, RepoOpenerEvent, ShortcutsHelp, ShortcutsHelpEvent,
    Sidebar, SidebarEvent, StashBranchDialog, StashBranchDialogEvent, SubmoduleView,
    SubmoduleViewEvent, TagDialog, TagDialogEvent, ToastKind, Toolbar, ToolbarEvent, WorktreeDialog,
    WorktreeDialogEvent,
};

use super::{ActiveOperation, BottomPanelMode, OperationOutput, UndoAction, UndoEntry, Workspace};

pub(super) fn build_worktree_graph_infos(
    worktrees: &[rgitui_git::WorktreeInfo],
) -> Vec<WorktreeGraphInfo> {
    worktrees
        .iter()
        .filter_map(|worktree| {
            let status = worktree.status.as_ref()?;
            let mut combined_breakdown = rgitui_graph::compute_breakdown(&status.staged);
            for (kind, count) in rgitui_graph::compute_breakdown(&status.unstaged) {
                *combined_breakdown.entry(kind).or_insert(0) += count;
            }
            Some(WorktreeGraphInfo {
                name: worktree.name.clone(),
                is_current: worktree.is_current,
                head_oid: worktree.head_oid,
                staged_count: status.staged.len(),
                unstaged_count: status.unstaged.len(),
                combined_breakdown,
                worktree_path: worktree.path.clone(),
                branch: worktree.branch.clone(),
            })
        })
        .collect()
}

fn active_worktree_status(
    tab: &super::ProjectTab,
    proj: &GitProject,
) -> (rgitui_git::WorkingTreeStatus, std::path::PathBuf) {
    if let Some(inspecting) = &tab.inspecting_worktree {
        let status = proj
            .worktrees()
            .iter()
            .find(|worktree| worktree.path == inspecting.path)
            .and_then(|worktree| worktree.status.clone())
            .unwrap_or_else(|| proj.status().clone());
        (status, inspecting.path.clone())
    } else {
        (proj.status().clone(), proj.repo_path().to_path_buf())
    }
}

/// Extracts a clean commit subject from a git stash message.
///
/// git stash produces messages like:
///   "WIP on main: abc1234 my last commit"
///   "On feature: def5678 WIP"
///   "On main: ghi9012"           ← no subject
///   custom message from -m
///
/// We strip the "WIP on <branch>: <sha>" or "On <branch>: <sha>" prefix
/// and return the meaningful subject. Falls back to "Stash" if the message
/// is blank.
fn extract_stash_summary(msg: &str) -> String {
    let first_line = msg.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        return String::from("Stash");
    }

    // Strip "WIP on <branch>: <sha> " or "On <branch>: <sha> " prefix.
    // The pattern: "WIP on <branch>: " or "On <branch>: " followed by a 7-char SHA.
    if let Some(colon_pos) = first_line.find(':') {
        let before_colon = &first_line[..colon_pos];
        let after_colon = first_line[colon_pos + 1..].trim_start();

        // Check if "before_colon" is a valid branch reference prefix.
        // Valid forms: "WIP on <branch>" or "On <branch>"
        let branch_part = if let Some(stripped) = before_colon.strip_prefix("WIP on ") {
            stripped
        } else if let Some(stripped) = before_colon.strip_prefix("On ") {
            stripped
        } else {
            before_colon // custom message, no prefix
        };

        // Parse "<sha> <subject>" or "<sha>" after the colon.
        // git stash uses 7-char short SHAs. The subject is everything after the space.
        // If there is no space, or the prefix before the space is not a valid SHA,
        // treat the entire remaining string as the subject.
        let remaining = after_colon.trim_start();
        let subject = if let Some(space_pos) = remaining.find(' ') {
            let prefix = &remaining[..space_pos];
            let after = remaining[space_pos..].trim_start();
            // Valid SHA: 7 hex chars. git stash short SHAs are 7 chars.
            if prefix.len() == 7 && prefix.chars().all(|c| c.is_ascii_hexdigit()) {
                if after.is_empty() {
                    None
                } else {
                    Some(after.to_string())
                }
            } else {
                // Not a standard SHA — treat whole remaining as subject.
                Some(remaining.to_string())
            }
        } else {
            // No space at all — no subject.
            None
        };

        return subject.unwrap_or_else(|| branch_part.to_string());
    }

    // No colon — custom message, use the first line as-is.
    first_line.to_string()
}

pub(super) fn update_commit_panel_for_active_worktree(
    workspace: &mut Workspace,
    cx: &mut Context<Workspace>,
) {
    let Some(tab) = workspace.tabs.get(workspace.active_tab) else {
        return;
    };
    let staged_count = {
        let proj = tab.project.read(cx);
        let (status, _) = active_worktree_status(tab, proj);
        status.staged.len()
    };
    tab.commit_panel.update(cx, |commit_panel, cx| {
        commit_panel.set_staged_count(staged_count, cx)
    });
}

pub(super) fn update_toolbar_for_active_worktree(
    workspace: &mut Workspace,
    cx: &mut Context<Workspace>,
) {
    let Some(tab) = workspace.tabs.get(workspace.active_tab) else {
        return;
    };
    let (has_changes, has_stashes, ahead, behind, has_github_token) = {
        let proj = tab.project.read(cx);
        let (status, _) = active_worktree_status(tab, proj);
        let has_changes = !status.staged.is_empty() || !status.unstaged.is_empty();
        let has_stashes = !proj.stashes().is_empty();
        let (ahead, behind) = proj
            .branches()
            .iter()
            .find(|branch| branch.is_head)
            .map(|branch| (branch.ahead, branch.behind))
            .unwrap_or((0, 0));
        let has_github_token = tab.prs_panel.read(cx).github_token().is_some();
        (has_changes, has_stashes, ahead, behind, has_github_token)
    };
    tab.toolbar.update(cx, |toolbar, cx| {
        toolbar.set_state(true, true, has_stashes, has_changes, has_github_token, cx);
        toolbar.set_ahead_behind(ahead, behind, cx);
    });
}

pub(super) fn update_sidebar_for_active_worktree(
    workspace: &mut Workspace,
    cx: &mut Context<Workspace>,
) {
    let Some(tab) = workspace.tabs.get(workspace.active_tab) else {
        return;
    };
    let (status, selected_path) = {
        let proj = tab.project.read(cx);
        active_worktree_status(tab, proj)
    };
    tab.sidebar.update(cx, |sidebar, cx| {
        sidebar.update_status(status.staged.clone(), status.unstaged.clone(), cx);
        sidebar.set_selected_worktree_by_path(Some(&selected_path), cx);
    });
    update_commit_panel_for_active_worktree(workspace, cx);
    update_toolbar_for_active_worktree(workspace, cx);
}

pub(super) fn subscribe_interactive_rebase(
    cx: &mut Context<Workspace>,
    interactive_rebase: &Entity<InteractiveRebase>,
) {
    cx.subscribe(
        interactive_rebase,
        |this, _ir, event: &InteractiveRebaseEvent, cx| match event {
            InteractiveRebaseEvent::Execute(entries) => {
                use crate::interactive_rebase::RebaseAction;

                if let Some(tab) = this.tabs.get(this.active_tab) {
                    let plan: Vec<RebasePlanEntry> = entries
                        .iter()
                        .map(|e| RebasePlanEntry {
                            oid: e.oid.clone(),
                            message: e.original_message.clone(),
                            action: match &e.action {
                                RebaseAction::Pick => RebaseEntryAction::Pick,
                                RebaseAction::Reword(msg) => {
                                    let m = if msg.is_empty() {
                                        e.original_message.clone()
                                    } else {
                                        msg.clone()
                                    };
                                    RebaseEntryAction::Reword(m)
                                }
                                RebaseAction::Squash => RebaseEntryAction::Squash,
                                RebaseAction::Fixup => RebaseEntryAction::Fixup,
                                RebaseAction::Drop => RebaseEntryAction::Drop,
                            },
                        })
                        .collect();

                    let project = tab.project.clone();
                    project.update(cx, |proj, cx| {
                        proj.rebase_interactive(plan, cx).detach();
                    });

                    let count = entries.len();
                    let msg = format!("Interactive rebase started with {} commits.", count);
                    this.set_status_message(msg.clone(), cx);
                    this.show_toast(msg, ToastKind::Info, cx);
                }
            }
            InteractiveRebaseEvent::Cancel => {
                cx.notify();
            }
        },
    )
    .detach();
}

pub(super) fn subscribe_ai(cx: &mut Context<Workspace>, ai: &Entity<AiGenerator>) {
    cx.subscribe(ai, |this, _ai, event: &AiEvent, cx| match event {
        AiEvent::GenerationCompleted(message) => {
            if let Some(tab) = this.tabs.get(this.active_tab) {
                let msg = message.clone();
                tab.commit_panel.update(cx, |cp, cx| {
                    cp.set_message(msg, cx);
                    cp.set_ai_generating(false, cx);
                });
            }
        }
        AiEvent::GenerationFailed(err) => {
            log::error!("AI generation failed: {}", err);
            let msg = format!("AI error: {}", err);
            this.set_status_message(msg.clone(), cx);
            this.show_toast(msg, ToastKind::Error, cx);
            if let Some(tab) = this.tabs.get(this.active_tab) {
                tab.commit_panel.update(cx, |cp, cx| {
                    cp.set_ai_generating(false, cx);
                });
            }
        }
        AiEvent::ToolCallStarted(description) => {
            this.set_status_message(format!("AI: {}", description), cx);
        }
        AiEvent::GenerationStarted => {
            this.set_status_message("Generating AI commit message...", cx);
            this.show_toast("Generating AI commit message...", ToastKind::Info, cx);
        }
    })
    .detach();
}

pub(super) fn subscribe_command_palette(
    cx: &mut Context<Workspace>,
    command_palette: &Entity<CommandPalette>,
) {
    cx.subscribe(
        command_palette,
        |this, _cp, event: &CommandPaletteEvent, cx| match event {
            CommandPaletteEvent::CommandSelected(cmd_id) => {
                this.execute_command(*cmd_id, cx);
            }
            CommandPaletteEvent::Dismissed => {}
        },
    )
    .detach();
}

pub(super) fn subscribe_branch_dialog(
    cx: &mut Context<Workspace>,
    branch_dialog: &Entity<BranchDialog>,
) {
    cx.subscribe(
        branch_dialog,
        |this, _bd, event: &BranchDialogEvent, cx| match event {
            BranchDialogEvent::CreateBranch { name, base_ref } => {
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    let project = tab.project.clone();
                    let name = name.clone();
                    let base = if base_ref.is_empty() {
                        None
                    } else {
                        Some(base_ref.as_str())
                    };
                    project.update(cx, |proj, cx| {
                        proj.create_branch_at(&name, base, cx).detach();
                    });
                }
                this.show_toast(format!("Branch '{}' created", name), ToastKind::Success, cx);
            }
            BranchDialogEvent::Dismissed => {}
        },
    )
    .detach();
}

pub(super) fn subscribe_tag_dialog(cx: &mut Context<Workspace>, tag_dialog: &Entity<TagDialog>) {
    cx.subscribe(
        tag_dialog,
        |this, _td, event: &TagDialogEvent, cx| match event {
            TagDialogEvent::CreateTag { name, target_oid } => {
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    let project = tab.project.clone();
                    let name = name.clone();
                    let oid = *target_oid;
                    project.update(cx, |proj, cx| {
                        proj.create_tag(&name, oid, cx).detach();
                    });
                }
            }
            TagDialogEvent::Dismissed => {}
        },
    )
    .detach();
}

pub(super) fn subscribe_stash_branch_dialog(
    cx: &mut Context<Workspace>,
    stash_branch_dialog: &Entity<StashBranchDialog>,
) {
    cx.subscribe(
        stash_branch_dialog,
        |this, _d, event: &StashBranchDialogEvent, cx| match event {
            StashBranchDialogEvent::CreateBranch { name, stash_index } => {
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    let project = tab.project.clone();
                    let name = name.clone();
                    let idx = *stash_index;
                    project.update(cx, |proj, cx| {
                        proj.stash_branch(&name, idx, cx).detach();
                    });
                }
                this.show_toast(
                    format!("Creating branch '{}' from stash #{}", name, stash_index),
                    ToastKind::Info,
                    cx,
                );
            }
            StashBranchDialogEvent::Dismissed => {}
        },
    )
    .detach();
}

pub(super) fn subscribe_create_pr_dialog(
    cx: &mut Context<Workspace>,
    create_pr_dialog: &Entity<CreatePrDialog>,
) {
    cx.subscribe(
        create_pr_dialog,
        |this, _d, event: &CreatePrDialogEvent, cx| match event {
            CreatePrDialogEvent::PrCreated { number, url } => {
                // Refresh the PR list to show the new PR
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    tab.prs_panel.update(cx, |panel, cx| {
                        panel.fetch_prs(cx);
                    });
                }
                this.show_toast(
                    format!("Pull request #{} created", number),
                    ToastKind::Success,
                    cx,
                );
                log::info!("Created PR #{}: {}", number, url);
            }
            CreatePrDialogEvent::Dismissed => {}
        },
    )
    .detach();
}

pub(super) fn subscribe_worktree_dialog(
    cx: &mut Context<Workspace>,
    worktree_dialog: &Entity<WorktreeDialog>,
) {
    cx.subscribe(
        worktree_dialog,
        |this, _wd, event: &WorktreeDialogEvent, cx| match event {
            WorktreeDialogEvent::CreateWorktree { name, path, branch } => {
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    let project = tab.project.clone();
                    let name = name.clone();
                    let path = std::path::PathBuf::from(path.clone());
                    let branch = branch.clone();
                    project.update(cx, |proj, cx| {
                        proj.create_worktree(name.clone(), path.clone(), branch.clone(), cx)
                            .detach();
                    });
                    this.show_toast(
                        format!("Creating worktree '{}' at '{}'", name, path.display()),
                        ToastKind::Info,
                        cx,
                    );
                }
            }
            WorktreeDialogEvent::Dismissed => {}
        },
    )
    .detach();
}

pub(super) fn subscribe_rename_dialog(
    cx: &mut Context<Workspace>,
    rename_dialog: &Entity<RenameDialog>,
) {
    cx.subscribe(
        rename_dialog,
        |this, _rd, event: &RenameDialogEvent, cx| match event {
            RenameDialogEvent::Rename { old_name, new_name } => {
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    let project = tab.project.clone();
                    let old = old_name.clone();
                    let new = new_name.clone();
                    project.update(cx, |proj, cx| {
                        proj.rename_branch(&old, &new, cx).detach();
                    });
                }
                this.show_toast(
                    format!("Branch renamed: {} -> {}", old_name, new_name),
                    ToastKind::Success,
                    cx,
                );
            }
            RenameDialogEvent::Dismissed => {}
        },
    )
    .detach();
}

pub(super) fn subscribe_confirm_dialog(
    cx: &mut Context<Workspace>,
    confirm_dialog: &Entity<ConfirmDialog>,
) {
    cx.subscribe(
        confirm_dialog,
        |this, _cd, event: &ConfirmDialogEvent, cx| match event {
            ConfirmDialogEvent::Confirmed(action) => {
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    let project = tab.project.clone();
                    match action {
                        ConfirmAction::DiscardFile(path) => {
                            let path_buf = std::path::PathBuf::from(path);
                            let worktree_path = this.effective_worktree_path(cx);
                            project.update(cx, |proj, cx| {
                                proj.discard_changes_at(&[path_buf], &worktree_path, cx)
                                    .detach();
                            });
                        }
                        ConfirmAction::ForcePush => {
                            project.update(cx, |proj, cx| {
                                proj.push_default(true, cx).detach();
                            });
                        }
                        ConfirmAction::BranchDelete(name) => {
                            let tip_oid = project
                                .read(cx)
                                .branches()
                                .iter()
                                .find(|b| b.name == *name)
                                .and_then(|b| b.tip_oid)
                                .map(|oid| oid.to_string());
                            let name = name.clone();
                            project.update(cx, |proj, cx| {
                                proj.delete_branch(&name, cx).detach();
                            });
                            if let Some(oid_hex) = tip_oid {
                                this.push_undo(
                                    UndoEntry {
                                        label: format!("Deleted branch '{}'", name),
                                        action: UndoAction::RecreateBranch { name, oid_hex },
                                        created_at: Instant::now(),
                                    },
                                    cx,
                                );
                                this.show_toast(
                                    "Branch deleted. Use command palette 'Undo' to restore.",
                                    ToastKind::Info,
                                    cx,
                                );
                            }
                        }
                        ConfirmAction::StashDrop(index) => {
                            let index = *index;
                            project.update(cx, |proj, cx| {
                                proj.stash_drop(index, cx).detach();
                            });
                        }
                        ConfirmAction::DiscardAll => {
                            let has_changes = project.read(cx).has_changes();
                            if has_changes {
                                project.update(cx, |proj, cx| {
                                    proj.stash_save(Some("rgitui-undo-discard"), cx).detach();
                                });
                                this.push_undo(
                                    UndoEntry {
                                        label: "Discarded all changes".into(),
                                        action: UndoAction::PopStash(0),
                                        created_at: Instant::now(),
                                    },
                                    cx,
                                );
                                this.show_toast(
                                    "Changes discarded. Use command palette 'Undo' to restore.",
                                    ToastKind::Info,
                                    cx,
                                );
                            }
                        }
                        ConfirmAction::CleanUntracked => {
                            project.update(cx, |proj, cx| {
                                proj.clean_untracked(cx).detach();
                            });
                        }
                        ConfirmAction::TagDelete(name) => {
                            let tag_oid = project
                                .read(cx)
                                .tags()
                                .iter()
                                .find(|t| t.name == *name)
                                .map(|t| t.oid.to_string());
                            let name = name.clone();
                            project.update(cx, |proj, cx| {
                                proj.delete_tag(&name, cx).detach();
                            });
                            if let Some(oid_hex) = tag_oid {
                                this.push_undo(
                                    UndoEntry {
                                        label: format!("Deleted tag '{}'", name),
                                        action: UndoAction::RecreateTag { name, oid_hex },
                                        created_at: Instant::now(),
                                    },
                                    cx,
                                );
                                this.show_toast(
                                    "Tag deleted. Use command palette 'Undo' to restore.",
                                    ToastKind::Info,
                                    cx,
                                );
                            }
                        }
                        ConfirmAction::ResetHard(target) => {
                            let previous_head_oid = project
                                .read(cx)
                                .recent_commits()
                                .first()
                                .map(|c| c.oid.to_string());
                            let target = target.clone();
                            project.update(cx, |proj, cx| {
                                if let Ok(oid) = git2::Oid::from_str(&target) {
                                    proj.reset_to_commit(oid, cx).detach();
                                } else {
                                    proj.reset_hard(cx).detach();
                                }
                            });
                            if let Some(oid_hex) = previous_head_oid {
                                this.push_undo(
                                    UndoEntry {
                                        label: format!(
                                            "Reset to {}",
                                            &target[..7.min(target.len())]
                                        ),
                                        action: UndoAction::ResetTo(oid_hex),
                                        created_at: Instant::now(),
                                    },
                                    cx,
                                );
                                this.show_toast(
                                    "Reset complete. Use command palette 'Undo' to revert.",
                                    ToastKind::Info,
                                    cx,
                                );
                            }
                        }
                        ConfirmAction::ResetSoft(target) => {
                            let target = target.clone();
                            project.update(cx, |proj, cx| {
                                if let Ok(oid) = git2::Oid::from_str(&target) {
                                    proj.reset_soft(oid, cx).detach();
                                }
                            });
                        }
                        ConfirmAction::ResetMixed(target) => {
                            let target = target.clone();
                            project.update(cx, |proj, cx| {
                                if let Ok(oid) = git2::Oid::from_str(&target) {
                                    proj.reset_mixed(oid, cx).detach();
                                }
                            });
                        }
                        ConfirmAction::RemoveRemote(name) => {
                            let name = name.clone();
                            project.update(cx, |proj, cx| {
                                proj.remove_remote(&name, cx).detach();
                            });
                        }
                        ConfirmAction::AbortMerge => {
                            project.update(cx, |proj, cx| {
                                proj.abort_operation(cx).detach();
                            });
                        }
                        ConfirmAction::WorktreeRemove(path) => {
                            let path = std::path::PathBuf::from(path.clone());
                            if let Some(tab) = this.tabs.get_mut(this.active_tab) {
                                if tab
                                    .inspecting_worktree
                                    .as_ref()
                                    .is_some_and(|inspecting| inspecting.path == path)
                                {
                                    tab.inspecting_worktree = None;
                                }
                            }
                            project.update(cx, |proj, cx| {
                                proj.remove_worktree(path, cx).detach();
                            });
                            update_sidebar_for_active_worktree(this, cx);
                        }
                    }
                }
            }
            ConfirmDialogEvent::Cancelled => {}
        },
    )
    .detach();
}

pub(super) fn subscribe_repo_opener(cx: &mut Context<Workspace>, repo_opener: &Entity<RepoOpener>) {
    cx.subscribe(
        repo_opener,
        |this, _ro, event: &RepoOpenerEvent, cx| match event {
            RepoOpenerEvent::OpenRepo(path) => {
                if let Err(e) = this.open_repo(path.clone(), cx) {
                    this.show_toast(format!("Failed to open: {}", e), ToastKind::Error, cx);
                } else {
                    this.refresh_all_tabs_prioritized(cx);
                }
            }
            RepoOpenerEvent::Dismissed => {
                this.focus.pending_focus_restore = true;
                cx.notify();
            }
        },
    )
    .detach();
}

pub(super) fn subscribe_shortcuts_help(
    cx: &mut Context<Workspace>,
    shortcuts_help: &Entity<ShortcutsHelp>,
) {
    cx.subscribe(
        shortcuts_help,
        |_this, _sh, _event: &ShortcutsHelpEvent, _cx| {},
    )
    .detach();
}

pub(super) fn subscribe_global_search(
    cx: &mut Context<Workspace>,
    global_search: &Entity<GlobalSearchView>,
) {
    // Clone once before subscribe so we can use the owned clone inside the async block.
    let gs_for_async = global_search.clone();
    cx.subscribe(
        global_search,
        move |this, gs, event: &GlobalSearchViewEvent, cx| match event {
            GlobalSearchViewEvent::SearchSubmit(query) => {
                let Some(project) = this.active_project().cloned() else {
                    return;
                };
                gs.update(cx, |g, cx| g.set_loading(true, cx));
                let gs_clone = gs_for_async.clone();
                let task = project.update(cx, |proj, cx| proj.git_grep_async(query.clone(), cx));
                cx.spawn(async move |_, cx: &mut gpui::AsyncApp| match task.await {
                    Ok(results) => {
                        cx.update(|cx| {
                            gs_clone.update(cx, |g, cx| g.set_results(results, cx));
                        });
                    }
                    Err(e) => {
                        cx.update(|cx| {
                            gs_clone.update(cx, |g, cx| {
                                g.set_error(format!("Search failed: {}", e), cx)
                            });
                        });
                    }
                })
                .detach();
            }
            GlobalSearchViewEvent::ResultSelected { path, line_number } => {
                // Dismiss search panel
                gs.update(cx, |g, cx| g.hide(cx));

                // Need an open tab to show the diff in
                if this.tabs.is_empty() {
                    return;
                }

                // Get repo_path before borrowing this mutably for the tab
                let repo_path = match this.active_project() {
                    Some(project) => project.read(cx).repo_path().to_path_buf(),
                    None => return,
                };

                let tab = &mut this.tabs[this.active_tab];
                tab.bottom_panel_mode = BottomPanelMode::Diff;
                cx.notify();

                let dv = tab.diff_viewer.clone();
                let dp = tab.detail_panel.clone();
                let path_buf = std::path::PathBuf::from(&path);
                let path_for_toast = path.clone();
                let line_for_toast = *line_number;

                cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                    let path_buf_owned = path_buf;
                    let path_str = path_buf_owned.to_string_lossy().to_string();
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            rgitui_git::compute_file_diff(&repo_path, &path_buf_owned, false)
                        })
                        .await;
                    cx.update(|cx| match result {
                        Ok(diff) => {
                            let path_str_owned = path_str.clone();
                            let line_owned = line_for_toast;
                            dv.update(cx, move |dv, cx| {
                                dv.set_diff(diff, path_str_owned, false, None, cx);
                                dv.scroll_to_line(line_owned, cx);
                            });
                            dp.update(cx, |dp, cx| dp.clear(cx));
                        }
                        Err(e) => {
                            log::error!("Failed to get diff for search result: {}", e);
                        }
                    });
                })
                .detach();

                this.show_toast(
                    format!("{}:{}", path_for_toast, line_for_toast),
                    ToastKind::Info,
                    cx,
                );
            }
            GlobalSearchViewEvent::Dismissed => {
                this.focus.pending_focus_restore = true;
                cx.notify();
            }
        },
    )
    .detach();
}

// ---- Per-tab subscriptions (called from open_repo) ----

pub(super) fn subscribe_project(
    cx: &mut Context<Workspace>,
    project: &Entity<GitProject>,
    graph: &Entity<GraphView>,
    sidebar: &Entity<Sidebar>,
    _commit_panel: &Entity<CommitPanel>,
    toolbar: &Entity<Toolbar>,
    diff_cache: Arc<Mutex<CommitDiffCache>>,
) {
    let graph = graph.clone();
    let sidebar = sidebar.clone();
    let toolbar = toolbar.clone();
    let diff_cache = diff_cache.clone();
    let has_prewarmed = Arc::new(std::sync::atomic::AtomicBool::new(false));

    cx.subscribe(project, {
        move |this, project, event: &GitProjectEvent, cx| match event {
            GitProjectEvent::AheadBehindRefreshed => {
                let proj = project.read(cx);
                let branches = proj.branches();
                let (ahead, behind) = branches
                    .iter()
                    .find(|b| b.is_head)
                    .map(|b| (b.ahead, b.behind))
                    .unwrap_or((0, 0));
                toolbar.update(cx, |tb, cx| {
                    tb.set_ahead_behind(ahead, behind, cx);
                });
            }
            GitProjectEvent::StatusChanged
            | GitProjectEvent::HeadChanged
            | GitProjectEvent::RefsChanged => {
                let (commits, has_more, branches, tags, remotes, stashes, worktrees, authors) = {
                    let proj = project.read(cx);
                    let commits = proj.recent_commits_arc();
                    let has_more = proj.has_more_commits();
                    let branches = proj.branches().to_vec();
                    let tags = proj.tags().to_vec();
                    let remotes = proj.remotes().to_vec();
                    let stashes = proj.stashes().to_vec();
                    let worktrees = proj.worktrees().to_vec();
                    let mut seen = std::collections::HashSet::new();
                    let authors: Vec<(String, String)> = commits
                        .iter()
                        .filter(|c| seen.insert(c.author.email.clone()))
                        .map(|c| (c.author.name.clone(), c.author.email.clone()))
                        .collect();
                    (
                        commits, has_more, branches, tags, remotes, stashes, worktrees, authors,
                    )
                };
                crate::avatar_resolver::resolve_avatars(authors, cx);

                let worktree_graph_infos = build_worktree_graph_infos(&worktrees);
                let worktrees_for_sidebar = worktrees.clone();
                graph.update(cx, |g, cx| {
                    g.set_commits(commits, cx);
                    g.set_all_loaded(!has_more);
                    g.set_worktree_statuses(worktree_graph_infos, cx);
                });

                sidebar.update(cx, |s, cx| {
                    s.update_branches(branches, cx);
                    s.update_tags(tags, cx);
                    s.update_remotes(remotes, cx);
                    s.update_stashes(stashes, cx);
                    s.update_worktrees(worktrees_for_sidebar, cx);
                });

                if let Some(tab) = this.tabs.get_mut(this.active_tab) {
                    if let Some(inspecting) = &tab.inspecting_worktree {
                        let inspected_worktree =
                            worktrees.iter().find(|wt| wt.path == inspecting.path);
                        let should_exit = match inspected_worktree {
                            None => true,
                            Some(worktree) => worktree.status.as_ref().is_some_and(|status| {
                                status.staged.is_empty() && status.unstaged.is_empty()
                            }),
                        };
                        if should_exit {
                            tab.inspecting_worktree = None;
                        }
                    }
                }

                update_sidebar_for_active_worktree(this, cx);

                // Pre-warm diff cache once for the first 30 commits.
                if has_prewarmed.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    // Already prewarmed — skip on subsequent StatusChanged events
                } else {
                    let proj = project.read(cx);
                    let prewarm_oids: Vec<git2::Oid> = {
                        let cached = diff_cache.lock().unwrap();
                        proj.recent_commits()
                            .iter()
                            .take(30)
                            .map(|c| c.oid)
                            .filter(|oid| !cached.contains(oid))
                            .collect()
                    };
                    if !prewarm_oids.is_empty() {
                        let repo_path = proj.repo_path().to_path_buf();
                        let prewarm_cache = diff_cache.clone();
                        log::debug!("diff_prewarm: starting for {} commits", prewarm_oids.len());
                        cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                            let tasks: Vec<_> = prewarm_oids
                                .into_iter()
                                .map(|oid| {
                                    let repo_path = repo_path.clone();
                                    cx.background_executor().spawn(async move {
                                        let result =
                                            rgitui_git::compute_commit_diff(&repo_path, oid);
                                        (oid, result)
                                    })
                                })
                                .collect();

                            let results = futures::future::join_all(tasks).await;
                            let ok_count = results.iter().filter(|(_, r)| r.is_ok()).count();
                            log::debug!("diff_prewarm: complete, {} results cached", ok_count);
                            let mut cache = prewarm_cache.lock().unwrap();
                            for (oid, result) in results {
                                if let Ok(commit_diff) = result {
                                    cache.insert(oid, Arc::new(commit_diff));
                                }
                            }
                        })
                        .detach();
                    }
                } // end prewarm gate
            }
            GitProjectEvent::OperationUpdated(update) => {
                let is_running = update.state == GitOperationState::Running;
                let operation_id = update.id;
                let failure_message = if let Some(details) = &update.details {
                    format!("{}: {}", update.summary, details)
                } else {
                    update.summary.clone()
                };

                match update.state {
                    GitOperationState::Running => {
                        this.operations.is_loading = true;
                        this.operations.loading_message = Some(update.summary.clone());
                        this.set_status_message(update.summary.clone(), cx);
                        this.operations.active_git_operation = Some(update.clone());
                        this.operations.active_operations.push(ActiveOperation {
                            id: operation_id,
                            label: update.summary.clone().into(),
                            started_at: Instant::now(),
                        });
                        this.show_toast(update.summary.clone(), ToastKind::Info, cx);
                    }
                    GitOperationState::Succeeded => {
                        this.operations
                            .active_operations
                            .retain(|op| op.id != operation_id);
                        this.operations.is_loading = !this.operations.active_operations.is_empty();
                        this.operations.loading_message = this
                            .operations
                            .active_operations
                            .last()
                            .map(|op| op.label.to_string());
                        this.set_status_message(update.summary.clone(), cx);
                        if this
                            .operations
                            .active_git_operation
                            .as_ref()
                            .is_some_and(|op| op.id == operation_id)
                        {
                            this.operations.active_git_operation = None;
                        }
                        if this
                            .operations
                            .last_failed_git_operation
                            .as_ref()
                            .is_some_and(|op| op.kind == update.kind)
                        {
                            this.operations.last_failed_git_operation = None;
                        }
                        let output_text = update.details.clone().unwrap_or_default();
                        if !output_text.is_empty() {
                            let now = Instant::now();
                            this.operations.last_operation_output = Some(OperationOutput {
                                operation: SharedString::from(
                                    update.kind.display_name().to_string(),
                                ),
                                output: output_text,
                                is_error: false,
                                timestamp: now,
                                expanded: false,
                            });
                            this.schedule_operation_output_auto_hide(now, cx);
                        }
                        this.show_toast(update.summary.clone(), ToastKind::Success, cx);
                    }
                    GitOperationState::Failed => {
                        this.operations
                            .active_operations
                            .retain(|op| op.id != operation_id);
                        this.operations.is_loading = !this.operations.active_operations.is_empty();
                        this.operations.loading_message = this
                            .operations
                            .active_operations
                            .last()
                            .map(|op| op.label.to_string());
                        if this
                            .operations
                            .active_git_operation
                            .as_ref()
                            .is_some_and(|op| op.id == operation_id)
                        {
                            this.operations.active_git_operation = None;
                        }
                        this.operations.last_failed_git_operation = Some(update.clone());
                        this.set_status_message(failure_message.clone(), cx);
                        let error_output = update
                            .details
                            .clone()
                            .unwrap_or_else(|| failure_message.clone());
                        this.operations.last_operation_output = Some(OperationOutput {
                            operation: SharedString::from(update.kind.display_name().to_string()),
                            output: error_output,
                            is_error: true,
                            timestamp: Instant::now(),
                            expanded: true,
                        });
                        this.show_toast(failure_message, ToastKind::Error, cx);
                    }
                }

                toolbar.update(cx, |tb, cx| {
                    tb.set_fetching(is_running && update.kind == GitOperationKind::Fetch, cx);
                    tb.set_pulling(is_running && update.kind == GitOperationKind::Pull, cx);
                    tb.set_pushing(is_running && update.kind == GitOperationKind::Push, cx);
                });
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_sidebar(
    cx: &mut Context<Workspace>,
    project: &Entity<GitProject>,
    sidebar: &Entity<Sidebar>,
    diff_viewer: &Entity<DiffViewer>,
    detail_panel: &Entity<DetailPanel>,
) {
    let project = project.clone();
    let diff_viewer = diff_viewer.clone();
    let detail_panel_ref = detail_panel.clone();
    let sidebar_owned = sidebar.clone();
    // Shadow the original parameter so the `move` closure doesn't capture it
    let sidebar = sidebar_owned;

    cx.subscribe(&sidebar, {
        move |this, _sidebar, event: &SidebarEvent, cx| match event {
            SidebarEvent::FileSelected { path, staged } => {
                log::info!("FileSelected: path={} staged={}", path, staged);
                let path_buf = std::path::PathBuf::from(path);
                let p = path.clone();
                let is_staged = *staged;
                let repo_path = this.effective_worktree_path(cx);
                let dv = diff_viewer.clone();
                let dp = detail_panel_ref.clone();

                // Prefetch blame + history in background for instant switching.
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    Workspace::prefetch_blame_and_history(
                        repo_path.clone(),
                        path.clone(),
                        tab.caches.clone(),
                        cx,
                    );
                }

                cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            rgitui_git::compute_file_diff(&repo_path, &path_buf, is_staged)
                        })
                        .await;
                    cx.update(|cx| match result {
                        Ok(diff) => {
                            dv.update(cx, |dv, cx| dv.set_diff(diff, p, is_staged, None, cx));
                            dp.update(cx, |dp, cx| dp.clear(cx));
                        }
                        Err(e) => log::error!("Failed to get diff: {}", e),
                    });
                })
                .detach();
            }
            SidebarEvent::ConflictFileSelected(path) => {
                let path_buf = std::path::PathBuf::from(path);
                let repo_path = this.effective_worktree_path(cx);
                let dv = diff_viewer.clone();
                cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            rgitui_git::compute_three_way_conflict_diff(&repo_path, &path_buf)
                        })
                        .await;
                    cx.update(|cx| match result {
                        Ok(three_way_diff) => {
                            dv.update(cx, |dv, cx| dv.set_three_way_diff(three_way_diff, cx));
                        }
                        Err(e) => log::error!("Failed to get 3-way conflict diff: {}", e),
                    });
                })
                .detach();
            }
            SidebarEvent::StageFile(path) => {
                let path_buf = std::path::PathBuf::from(path);
                let worktree_path = this.effective_worktree_path(cx);
                project.update(cx, |proj, cx| {
                    proj.stage_files_at(&[path_buf], &worktree_path, cx)
                        .detach();
                });
            }
            SidebarEvent::UnstageFile(path) => {
                let path_buf = std::path::PathBuf::from(path);
                let worktree_path = this.effective_worktree_path(cx);
                project.update(cx, |proj, cx| {
                    proj.unstage_files_at(&[path_buf], &worktree_path, cx)
                        .detach();
                });
            }
            SidebarEvent::StageAll => {
                let worktree_path = this.effective_worktree_path(cx);
                project.update(cx, |proj, cx| {
                    proj.stage_all_at(&worktree_path, cx).detach();
                });
            }
            SidebarEvent::UnstageAll => {
                let worktree_path = this.effective_worktree_path(cx);
                project.update(cx, |proj, cx| {
                    proj.unstage_all_at(&worktree_path, cx).detach();
                });
            }
            SidebarEvent::BranchCheckout(name) => {
                let name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.checkout_branch(&name, cx).detach();
                });
            }
            SidebarEvent::RemoteFetch(name) => {
                let name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.fetch(&name, cx).detach();
                });
            }
            SidebarEvent::RemotePull(name) => {
                let name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.pull(&name, cx).detach();
                });
            }
            SidebarEvent::RemotePush(name) => {
                let name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.push(&name, false, cx).detach();
                });
            }
            SidebarEvent::RemoteRemove(name) => {
                let name = name.clone();
                this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Remove Remote",
                        format!(
                            "Remove remote '{}' and its configured URLs from this repository?",
                            name
                        ),
                        ConfirmAction::RemoveRemote(name),
                        cx,
                    );
                });
            }
            SidebarEvent::DiscardFile(path) => {
                let path = path.clone();
                this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Discard Changes",
                        format!("Are you sure you want to discard changes to {}?", path),
                        ConfirmAction::DiscardFile(path),
                        cx,
                    );
                });
            }
            SidebarEvent::AcceptConflictOurs(path) => {
                let path = path.clone();
                project.update(cx, |proj, cx| {
                    proj.accept_conflict_ours(path, cx).detach();
                });
            }
            SidebarEvent::AcceptConflictTheirs(path) => {
                let path = path.clone();
                project.update(cx, |proj, cx| {
                    proj.accept_conflict_theirs(path, cx).detach();
                });
            }
            SidebarEvent::StashSelected(index) => {
                let idx = *index;
                let repo_path = project.read(cx).repo_path().to_path_buf();
                let dv = diff_viewer.clone();
                let dp = detail_panel_ref.clone();
                // Capture stash metadata before spawning so we can build a synthetic CommitInfo
                let stash_info = project
                    .read(cx)
                    .stashes()
                    .get(idx)
                    .map(|s| (s.message.clone(), s.oid));
                cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { rgitui_git::compute_stash_diff(&repo_path, idx) })
                        .await;
                    cx.update(|cx| match result {
                        Ok(commit_diff) => {
                            // Show first file in diff viewer immediately
                            if let Some(first_file) = commit_diff.files.first() {
                                let path = first_file.path.display().to_string();
                                dv.update(cx, |dv, cx| {
                                    dv.set_diff(first_file.clone(), path, false, None, cx)
                                });
                            }
                            // Populate detail panel with the full stash file list so users
                            // can click any file in the stash to view its diff.
                            if let Some((msg, oid)) = stash_info {
                                let unknown_sig = Signature {
                                    name: String::from("stash"),
                                    email: String::new(),
                                };
                                // git stash messages are "WIP on <branch>: <sha> <subject>" or
                                // "On <branch>: <sha> <subject>". We want just the subject.
                                // Strip the WIP/On prefix and the trailing hash+subject.
                                let summary = extract_stash_summary(&msg);
                                let synthetic = CommitInfo {
                                    oid,
                                    short_id: oid.to_string()[..7].to_string(),
                                    summary,
                                    message: msg,
                                    author: unknown_sig.clone(),
                                    committer: unknown_sig,
                                    co_authors: vec![],
                                    time: chrono::Utc::now(),
                                    parent_oids: vec![],
                                    refs: vec![],
                                    is_signed: false,
                                };
                                dp.update(cx, |dp, cx| dp.set_commit(synthetic, commit_diff, cx));
                            } else {
                                dp.update(cx, |dp, cx| dp.clear(cx));
                            }
                        }
                        Err(e) => log::error!("Failed to get stash diff: {}", e),
                    });
                })
                .detach();
            }
            SidebarEvent::TagSelected(name) => {
                let proj = project.read(cx);
                if let Ok(oid) = proj.resolve_tag_to_oid(name) {
                    if let Some(tab) = this.tabs.get(this.active_tab) {
                        tab.graph.update(cx, |g, cx| {
                            g.scroll_to_commit(oid, cx);
                        });
                    }
                } else {
                    log::warn!("Could not resolve tag '{}' to a commit", name);
                }
            }
            SidebarEvent::WorktreeSelected(index) => {
                let worktrees = project.read(cx).worktrees().to_vec();
                if let Some(wt) = worktrees.get(*index) {
                    if let Some(tab) = this.tabs.get_mut(this.active_tab) {
                        tab.inspecting_worktree = if wt.is_current {
                            None
                        } else {
                            Some(super::InspectingWorktree {
                                name: wt.name.clone(),
                                path: wt.path.clone(),
                                branch: wt.branch.clone(),
                            })
                        };
                    }
                    if let Some(oid) = wt.head_oid {
                        if let Some(tab) = this.tabs.get(this.active_tab) {
                            tab.graph.update(cx, |g, cx| {
                                g.scroll_to_commit(oid, cx);
                            });
                        }
                    }
                    update_sidebar_for_active_worktree(this, cx);
                    if let Some(tab) = this.tabs.get(this.active_tab) {
                        let dp = tab.detail_panel.clone();
                        let dv = tab.diff_viewer.clone();
                        dp.update(cx, |dp, cx| dp.clear(cx));
                        dv.update(cx, |dv, cx| dv.clear(cx));
                    }
                }
            }
            SidebarEvent::WorktreeCreate => {
                let branch = project.read(cx).head_branch().map(|s| s.to_string());
                this.dialogs.worktree_dialog.update(cx, |wd, cx| {
                    wd.show_visible(branch, cx);
                });
            }
            SidebarEvent::WorktreeRemove(index) => {
                let worktrees = project.read(cx).worktrees().to_vec();
                if let Some(wt) = worktrees.get(*index) {
                    if wt.is_current {
                        this.show_toast(
                            "Cannot remove the current worktree".to_string(),
                            ToastKind::Error,
                            cx,
                        );
                    } else {
                        let path_display = wt.path.display().to_string();
                        let path = wt.path.clone();
                        this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                            cd.show_visible(
                                "Remove Worktree",
                                format!(
                                    "Are you sure you want to remove worktree '{}'? \
                                     This will delete the working tree at '{}'.",
                                    wt.name, path_display
                                ),
                                ConfirmAction::WorktreeRemove(path.to_string_lossy().to_string()),
                                cx,
                            );
                        });
                    }
                }
            }
            SidebarEvent::BranchSelected(name) => {
                let proj = project.read(cx);
                if let Ok(oid) = proj.resolve_branch_to_oid(name) {
                    if let Some(tab) = this.tabs.get(this.active_tab) {
                        tab.graph.update(cx, |g, cx| {
                            g.scroll_to_commit(oid, cx);
                        });
                    }
                } else {
                    log::warn!("Could not resolve branch '{}' to a commit", name);
                }
            }
            SidebarEvent::BranchCopyName(name) => {
                let name = name.clone();
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(name.clone()));
                this.show_toast(format!("Copied: {}", name), ToastKind::Info, cx);
            }
            SidebarEvent::MergeBranch(name) => {
                let name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.merge_branch(&name, cx).detach();
                });
            }
            SidebarEvent::BranchCreate => {
                this.dialogs.branch_dialog.update(cx, |bd, cx| {
                    bd.show_visible(None, cx);
                });
            }
            SidebarEvent::BranchDelete(name) => {
                let name = name.clone();
                this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Delete Branch",
                        format!("Are you sure you want to delete branch '{}'?", name),
                        ConfirmAction::BranchDelete(name),
                        cx,
                    );
                });
            }
            SidebarEvent::OpenRepo => {
                this.overlays.repo_opener.update(cx, |ro, cx| {
                    ro.toggle_visible(cx);
                });
            }
            SidebarEvent::TagCheckout(name) => {
                let name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.checkout_tag(&name, cx).detach();
                });
            }
            SidebarEvent::TagDelete(name) => {
                let name = name.clone();
                this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Delete Tag",
                        format!(
                            "Are you sure you want to delete tag '{}'? This cannot be undone.",
                            name
                        ),
                        ConfirmAction::TagDelete(name),
                        cx,
                    );
                });
            }
            SidebarEvent::StashApply(index) => {
                let index = *index;
                project.update(cx, |proj, cx| {
                    proj.stash_apply(index, cx).detach();
                });
            }
            SidebarEvent::BranchRename(name) => {
                let name = name.clone();
                this.dialogs.rename_dialog.update(cx, |rd, cx| {
                    rd.show_visible(name, cx);
                });
            }
            SidebarEvent::StashDrop(index) => {
                let index = *index;
                this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Drop Stash",
                        format!(
                            "Are you sure you want to drop stash@{{{}}}? This cannot be undone.",
                            index
                        ),
                        ConfirmAction::StashDrop(index),
                        cx,
                    );
                });
            }
            SidebarEvent::StashPop(index) => {
                let index = *index;
                project.update(cx, |proj, cx| {
                    proj.stash_pop(index, cx).detach();
                });
            }
            SidebarEvent::StashBranch(index) => {
                let index = *index;
                this.dialogs.stash_branch_dialog.update(cx, |d, cx| {
                    d.show_visible(index, cx);
                });
            }
            SidebarEvent::ToggleDir(dir_key) => {
                let (prefix, dir) = dir_key.split_once(':').unwrap_or(("", ""));
                _sidebar.update(cx, |s, cx| {
                    s.toggle_dir(prefix, dir, cx);
                });
            }
        }
    })
    .detach();
}

type CommitDiffCache = LruCache<git2::Oid, Arc<rgitui_git::CommitDiff>>;

pub(super) fn subscribe_graph(
    cx: &mut Context<Workspace>,
    project: &Entity<GitProject>,
    graph: &Entity<GraphView>,
    diff_viewer: &Entity<DiffViewer>,
    detail_panel: &Entity<DetailPanel>,
    diff_cache: Arc<Mutex<CommitDiffCache>>,
) {
    let project = project.clone();
    let diff_viewer = diff_viewer.clone();
    let detail_panel_ref = detail_panel.clone();

    let graph_entity = graph.clone();

    cx.subscribe(graph, {
        move |this, _graph, event: &GraphViewEvent, cx| {
            match event {
                GraphViewEvent::CommitSelected(oid) => {
                    let commit_oid = *oid;
                    log::info!("CommitSelected: oid={:.7}", commit_oid);

                    // Extract everything we need from the project before
                    // releasing the immutable borrow on cx.
                    let (commit_info, repo_path, prefetch_oids) = {
                        let proj = project.read(cx);
                        let commits = proj.recent_commits();
                        let info = commits.iter().find(|c| c.oid == commit_oid).cloned();
                        let path = proj.repo_path().to_path_buf();

                        const PREFETCH_WINDOW: usize = 25;
                        let mut oids = Vec::new();
                        if let Some(idx) = commits.iter().position(|c| c.oid == commit_oid) {
                            for delta in 1..=PREFETCH_WINDOW {
                                if let Some(prev_idx) = idx.checked_sub(delta) {
                                    if let Some(c) = commits.get(prev_idx) {
                                        oids.push(c.oid);
                                    }
                                }
                                if let Some(next_idx) = idx.checked_add(delta) {
                                    if let Some(c) = commits.get(next_idx) {
                                        oids.push(c.oid);
                                    }
                                }
                            }
                        }
                        (info, path, oids)
                    };

                    // Check cache synchronously — instant display on cache hit.
                    let cached = diff_cache.lock().unwrap().get(&commit_oid);
                    log::debug!("CommitSelected: diff_cache {}", if cached.is_some() { "hit" } else { "miss" });
                    if let Some(cached) = cached {
                        let dv = diff_viewer.clone();
                        let dp = detail_panel_ref.clone();
                        let cached = cached.clone();
                        if let Some(first_file) = cached.files.first() {
                            let path = first_file.path.display().to_string();
                            let oid_str = commit_oid.to_string();
                            dv.update(cx, |dv, cx| {
                                dv.set_diff(first_file.clone(), path, false, Some(&oid_str), cx)
                            });
                        }
                        // Enrich commit info (is_signed, co_authors) in background
                        if let Some(mut info) = commit_info.clone() {
                            let enrich_path = repo_path.clone();
                            cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                                let enriched = cx
                                    .background_executor()
                                    .spawn(async move {
                                        rgitui_git::enrich_commit_info(&enrich_path, commit_oid)
                                    })
                                    .await;
                                if let Ok((is_signed, co_authors)) = enriched {
                                    info.is_signed = is_signed;
                                    info.co_authors = co_authors;
                                }
                                cx.update(|cx| {
                                    dp.update(cx, |dp, cx| {
                                        dp.set_commit(info, (*cached).clone(), cx)
                                    });
                                });
                            })
                            .detach();
                        }
                    } else {
                        // Cache miss — submit selected commit first so it gets
                        // a thread pool slot before the prefetch tasks.
                        let dv = diff_viewer.clone();
                        let dp = detail_panel_ref.clone();
                        let cache = diff_cache.clone();
                        let oid_str = commit_oid.to_string();
                        let enrich_path = repo_path.clone();

                        cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                            // Compute diff and enrich commit info in parallel
                            let diff_task = cx
                                .background_executor()
                                .spawn(async move {
                                    rgitui_git::compute_commit_diff(&repo_path, commit_oid)
                                });
                            let enrich_task = cx
                                .background_executor()
                                .spawn(async move {
                                    rgitui_git::enrich_commit_info(&enrich_path, commit_oid)
                                });

                            let diff_result = diff_task.await;
                            let enrich_result = enrich_task.await;

                            cx.update(|cx| match diff_result {
                                Ok(commit_diff) => {
                                    let diff_arc = Arc::new(commit_diff.clone());
                                    cache.lock().unwrap().insert(commit_oid, diff_arc.clone());

                                    if let Some(mut info) = commit_info {
                                        if let Ok((is_signed, co_authors)) = enrich_result {
                                            info.is_signed = is_signed;
                                            info.co_authors = co_authors;
                                        }
                                        dp.update(cx, |dp, cx| {
                                            dp.set_commit(info, commit_diff.clone(), cx)
                                        });
                                    }
                                    if let Some(first_file) = commit_diff.files.first() {
                                        let path = first_file.path.display().to_string();
                                        dv.update(cx, |dv, cx| {
                                            dv.set_diff(first_file.clone(), path, false, Some(&oid_str), cx)
                                        });
                                    }
                                }
                                Err(e) => log::error!("Failed to get commit diff: {}", e),
                            });
                        })
                        .detach();
                    }

                    // Prefetch neighbors — runs in both cache hit and miss cases.
                    // On miss, this fires AFTER the selected commit task is queued,
                    // so the selected commit gets a thread pool slot first.
                    if !prefetch_oids.is_empty() {
                        log::debug!("diff_prefetch: starting for {} OIDs around {:.7}", prefetch_oids.len(), commit_oid);
                        let prefetch_repo_path = project.read(cx).repo_path().to_path_buf();
                        let prefetch_cache = diff_cache.clone();
                        cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                            let oids_to_fetch: Vec<git2::Oid> = {
                                let cached = prefetch_cache.lock().unwrap();
                                prefetch_oids
                                    .into_iter()
                                    .filter(|oid| !cached.contains(oid))
                                    .collect()
                            };

                            let tasks: Vec<_> = oids_to_fetch
                                .into_iter()
                                .map(|oid| {
                                    let repo_path = prefetch_repo_path.clone();
                                    cx.background_executor().spawn(async move {
                                        let result =
                                            rgitui_git::compute_commit_diff(&repo_path, oid);
                                        (oid, result)
                                    })
                                })
                                .collect();

                            let results = futures::future::join_all(tasks).await;

                            log::debug!("diff_prefetch: complete, {} results cached", results.iter().filter(|(_, r)| r.is_ok()).count());
                            let mut cache = prefetch_cache.lock().unwrap();
                            for (oid, result) in results {
                                if let Ok(commit_diff) = result {
                                    cache.insert(oid, Arc::new(commit_diff));
                                }
                            }
                        })
                        .detach();
                    }
                }
                GraphViewEvent::CherryPick(oid) => {
                    let oid = *oid;
                    // Capture HEAD before cherry-pick for undo support
                    let previous_head_oid: Option<String> = project
                        .read(cx)
                        .recent_commits()
                        .iter()
                        .find(|c| {
                            c.refs
                                .iter()
                                .any(|r| matches!(r, rgitui_git::RefLabel::Head))
                        })
                        .map(|c| c.oid.to_string());

                    project.update(cx, |proj, cx| {
                        proj.cherry_pick(oid, cx).detach();
                    });

                    if let Some(oid_hex) = previous_head_oid {
                        let label = format!(
                            "Cherry-pick {}",
                            &oid_hex[..7.min(oid_hex.len())]
                        );
                        this.push_undo(
                            UndoEntry {
                                label,
                                action: UndoAction::ResetTo(oid_hex),
                                created_at: Instant::now(),
                            },
                            cx,
                        );
                    }
                }
                GraphViewEvent::RevertCommit(oid) => {
                    let oid = *oid;
                    // Capture HEAD before revert for undo support
                    let previous_head_oid: Option<String> = project
                        .read(cx)
                        .recent_commits()
                        .iter()
                        .find(|c| {
                            c.refs
                                .iter()
                                .any(|r| matches!(r, rgitui_git::RefLabel::Head))
                        })
                        .map(|c| c.oid.to_string());

                    project.update(cx, |proj, cx| {
                        proj.revert_commit(oid, cx).detach();
                    });

                    if let Some(oid_hex) = previous_head_oid {
                        let label = format!("Revert {}", &oid_hex[..7.min(oid_hex.len())]);
                        this.push_undo(
                            UndoEntry {
                                label,
                                action: UndoAction::ResetTo(oid_hex),
                                created_at: Instant::now(),
                            },
                            cx,
                        );
                    }
                }
                GraphViewEvent::CreateBranchAtCommit(oid) => {
                    let sha = oid.to_string();
                    this.dialogs.branch_dialog.update(cx, |bd, cx| {
                        bd.show_visible(Some(sha), cx);
                    });
                }
                GraphViewEvent::CheckoutCommit(oid) => {
                    let oid = *oid;
                    project.update(cx, |proj, cx| {
                        proj.checkout_commit(oid, cx).detach();
                    });
                }
                GraphViewEvent::CopyCommitSha(sha) => {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(sha.clone()));
                    let short = &sha[..7.min(sha.len())];
                    this.show_toast(
                        format!("Copied SHA: {}", short),
                        ToastKind::Success,
                        cx,
                    );
                }
                GraphViewEvent::CopyCommitMessage(msg) => {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(msg.clone()));
                    // Show first line of message as confirmation
                    let first_line = msg.lines().next().unwrap_or(msg);
                    let preview = if first_line.len() > 40 {
                        format!("{}...", &first_line[..40])
                    } else {
                        first_line.to_string()
                    };
                    this.show_toast(
                        format!("Copied: {}", preview),
                        ToastKind::Success,
                        cx,
                    );
                }
                GraphViewEvent::CopyAuthorName(name) => {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(name.clone()));
                    this.show_toast(format!("Copied author: {}", name), ToastKind::Success, cx);
                }
                GraphViewEvent::CopyDate(date) => {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(date.clone()));
                    this.show_toast(format!("Copied date: {}", date), ToastKind::Success, cx);
                }
                GraphViewEvent::ViewOnGithub(oid) => {
                    let oid = *oid;
                    let remotes = project.read(cx).remotes();
                    let remote_url = remotes
                        .iter()
                        .find(|r| r.name == "origin")
                        .or_else(|| remotes.first())
                        .and_then(|r| r.url.clone());

                    if let Some(url) = remote_url {
                        if let Some((owner, repo)) =
                            crate::issues_panel::parse_github_owner_repo(&url)
                        {
                            let github_url =
                                format!("https://github.com/{}/{}/commit/{}", owner, repo, oid);
                            cx.open_url(&github_url);
                            this.show_toast(
                                format!("Opening {} on GitHub", &oid.to_string()[..7]),
                                ToastKind::Info,
                                cx,
                            );
                        } else {
                            this.show_toast(
                                "Remote URL is not a known Git host. Cannot open commit.",
                                ToastKind::Warning,
                                cx,
                            );
                        }
                    } else {
                        this.show_toast(
                            "No remote configured. Add a remote to open commits on GitHub.",
                            ToastKind::Warning,
                            cx,
                        );
                    }
                }
                GraphViewEvent::CreateTagAtCommit(oid) => {
                    let oid = *oid;
                    this.dialogs.tag_dialog.update(cx, |td, cx| {
                        td.show_visible(oid, cx);
                    });
                }
                GraphViewEvent::ResetToCommit(oid, sha) => {
                    let oid = *oid;
                    let sha = sha.clone();
                    let short = &sha[..7.min(sha.len())];
                    this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                        cd.show_visible(
                            "Reset to Commit",
                            format!(
                                "Hard reset the current branch to {}? All uncommitted changes and commits after this point will be lost.",
                                short
                            ),
                            ConfirmAction::ResetHard(oid.to_string()),
                            cx,
                        );
                    });
                }
                GraphViewEvent::BisectGood(oid) => {
                    let oid = *oid;
                    let state = project.read(cx).repo_state();
                    if !matches!(state, rgitui_git::RepoState::Bisect) {
                        this.show_toast("No bisect in progress. Use 'Bisect Start' first.", ToastKind::Warning, cx);
                    } else {
                        project.update(cx, |proj, cx| {
                            proj.bisect_good(Some(oid), cx).detach();
                        });
                    }
                }
                GraphViewEvent::BisectBad(oid) => {
                    let oid = *oid;
                    let state = project.read(cx).repo_state();
                    if !matches!(state, rgitui_git::RepoState::Bisect) {
                        this.show_toast("No bisect in progress. Use 'Bisect Start' first.", ToastKind::Warning, cx);
                    } else {
                        project.update(cx, |proj, cx| {
                            proj.bisect_bad(Some(oid), cx).detach();
                        });
                    }
                }
                GraphViewEvent::LoadMoreCommits => {
                    let already_loaded = project.read(cx).loaded_commit_count();
                    this.show_toast(
                        format!("Loading more commits ({} loaded so far)...", already_loaded),
                        ToastKind::Info,
                        cx,
                    );
                    project.update(cx, |proj, cx| {
                        proj.load_more_commits(cx).detach();
                    });
                }
                GraphViewEvent::ToggleMyCommits => {
                    let is_active = graph_entity.update(cx, |g, _| g.my_commits_active());
                    let user_email = project.read(cx).current_user_email().map(String::from);
                    let already_loaded = project.read(cx).loaded_commit_count();
                    let msg = if is_active {
                        format!("Filtering to your commits ({} loaded)...", already_loaded)
                    } else {
                        "Showing all commits.".into()
                    };
                    this.show_toast(&msg, ToastKind::Info, cx);
                    project.update(cx, |proj, cx| {
                        if is_active {
                            proj.set_commit_author_filter(user_email);
                        } else {
                            proj.set_commit_author_filter(None);
                        }
                        proj.load_more_commits(cx).detach();
                    });
                }
                GraphViewEvent::WorktreeNodeSelected {
                    worktree_path,
                    name,
                } => {
                    let worktree_path = worktree_path.clone();
                    let name = name.clone();
                    let worktree = project
                        .read(cx)
                        .worktrees()
                        .iter()
                        .find(|worktree| worktree.path == worktree_path)
                        .cloned();
                    if let Some(tab) = this.tabs.get_mut(this.active_tab) {
                        tab.inspecting_worktree = worktree.and_then(|worktree| {
                            if worktree.is_current {
                                None
                            } else {
                                Some(super::InspectingWorktree {
                                    name,
                                    path: worktree_path,
                                    branch: worktree.branch,
                                })
                            }
                        });
                    }
                    update_sidebar_for_active_worktree(this, cx);
                    let dp = detail_panel_ref.clone();
                    let dv = diff_viewer.clone();
                    dp.update(cx, |dp, cx| dp.clear(cx));
                    dv.update(cx, |dv, cx| dv.clear(cx));
                }
                GraphViewEvent::InteractiveRebase(target_oid) => {
                    let target = *target_oid;
                    let commits = project.read(cx).recent_commits().to_vec();

                    // Find the index of the target commit in the loaded commits list.
                    // The interactive rebase will include all commits from HEAD (index 0)
                    // down to and including this target.
                    let Some(target_idx) = commits.iter().position(|c| c.oid == target) else {
                        this.show_toast(
                            "Selected commit not in loaded commits. Load more and try again.",
                            ToastKind::Warning,
                            cx,
                        );
                        return;
                    };

                    // Build the entry list from HEAD down to (and including) the target.
                    // The interactive rebase editor will rebasing onto the parent of the
                    // first entry (i.e., commits[0]..=commits[target_idx]).
                    let head_branch = project
                        .read(cx)
                        .head_branch()
                        .unwrap_or("HEAD")
                        .to_string();
                    let base_short = if target_idx > 0 {
                        commits
                            .get(target_idx)
                            .map(|c| c.short_id.as_str())
                            .unwrap_or("HEAD")
                            .to_string()
                    } else {
                        head_branch.clone()
                    };

                    let entries: Vec<crate::interactive_rebase::RebaseEntry> = commits
                        [..=target_idx]
                        .iter()
                        .map(|c| crate::interactive_rebase::RebaseEntry {
                            oid: c.oid.to_string(),
                            original_message: c.summary.clone(),
                            author: c.author.name.clone(),
                            action: crate::interactive_rebase::RebaseAction::Pick,
                        })
                        .collect();

                    if entries.is_empty() {
                        this.show_toast(
                            "No commits available for interactive rebase.",
                            ToastKind::Warning,
                            cx,
                        );
                        return;
                    }

                    // Use the base short oid as the "onto" target ref in the dialog.
                    this.overlays
                        .interactive_rebase
                        .update(cx, |ir, cx| {
                            ir.show_visible(entries, format!("{} (rebase onto)", base_short), cx);
                        });
                }
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_detail_panel(
    cx: &mut Context<Workspace>,
    project: &Entity<GitProject>,
    diff_viewer: &Entity<DiffViewer>,
    detail_panel: &Entity<DetailPanel>,
) {
    let project = project.clone();
    let diff_viewer = diff_viewer.clone();
    let detail_panel_cloned = detail_panel.clone();

    cx.subscribe(detail_panel, {
        move |this, _dp, event: &DetailPanelEvent, cx| match event {
            DetailPanelEvent::FileSelected(file_diff, path) => {
                let p = path.clone();
                let fd = file_diff.clone();
                let oid_str = _dp.read(cx).commit().map(|c| c.oid.to_string());
                diff_viewer.update(cx, |dv, cx| {
                    dv.set_diff(fd, p, false, oid_str.as_deref(), cx);
                });
            }
            DetailPanelEvent::CopySha(sha) => {
                let short = &sha[..7.min(sha.len())];
                this.show_toast(format!("Copied SHA: {}", short), ToastKind::Success, cx);
            }
            DetailPanelEvent::CherryPick(sha) => {
                if let Some(project) = this.active_project().cloned() {
                    if let Ok(oid) = git2::Oid::from_str(sha) {
                        // Capture HEAD before cherry-pick for undo support
                        let previous_head_oid: Option<String> = project
                            .read(cx)
                            .recent_commits()
                            .iter()
                            .find(|c| {
                                c.refs
                                    .iter()
                                    .any(|r| matches!(r, rgitui_git::RefLabel::Head))
                            })
                            .map(|c| c.oid.to_string());

                        project.update(cx, |proj, cx| {
                            proj.cherry_pick(oid, cx).detach();
                        });

                        if let Some(oid_hex) = previous_head_oid {
                            let label = format!("Cherry-pick {}", &oid_hex[..7.min(oid_hex.len())]);
                            this.push_undo(
                                UndoEntry {
                                    label,
                                    action: UndoAction::ResetTo(oid_hex),
                                    created_at: Instant::now(),
                                },
                                cx,
                            );
                        }
                    }
                }
            }
            DetailPanelEvent::NavigatePrevCommit | DetailPanelEvent::NavigateNextCommit => {
                let is_prev = matches!(event, DetailPanelEvent::NavigatePrevCommit);
                let proj = project.read(cx);
                let commits = proj.recent_commits();
                if commits.is_empty() {
                    return;
                }
                // Get current commit OID from detail panel's displayed commit
                let Some(current_oid) = detail_panel_cloned.read(cx).commit().map(|c| c.oid) else {
                    return;
                };
                let Some(pos) = commits.iter().position(|c| c.oid == current_oid) else {
                    return;
                };
                let target_pos = if is_prev {
                    pos.saturating_sub(1) // older = higher index (next older)
                } else {
                    pos.saturating_add(1) // newer = lower index (next newer)
                };
                if target_pos >= commits.len() || target_pos == pos {
                    return;
                }
                let target_oid = commits[target_pos].oid;
                let _ = proj;

                // Re-fetch commit info and diff for the target
                let repo_path = project.read(cx).repo_path().to_path_buf();
                let dv = diff_viewer.clone();
                let dp = detail_panel_cloned.clone();
                let project_for_async = project.clone();
                cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                    let result = cx
                        .background_executor()
                        .spawn(
                            async move { rgitui_git::compute_commit_diff(&repo_path, target_oid) },
                        )
                        .await;
                    cx.update(|cx| {
                        let commit_info = project_for_async
                            .read(cx)
                            .recent_commits()
                            .iter()
                            .find(|c| c.oid == target_oid)
                            .cloned();
                        if let Some(info) = commit_info {
                            if let Ok(commit_diff) = result {
                                dp.update(cx, |dp, cx| {
                                    dp.set_commit(info.clone(), commit_diff.clone(), cx)
                                });
                                if let Some(first_file) = commit_diff.files.first() {
                                    let path = first_file.path.display().to_string();
                                    let oid_str = target_oid.to_string();
                                    dv.update(cx, |dv, cx| {
                                        dv.set_diff(
                                            first_file.clone(),
                                            path,
                                            false,
                                            Some(&oid_str),
                                            cx,
                                        )
                                    });
                                }
                            }
                        }
                    });
                })
                .detach();
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_diff_viewer(
    cx: &mut Context<Workspace>,
    project: &Entity<GitProject>,
    diff_viewer: &Entity<DiffViewer>,
) {
    let project = project.clone();
    let diff_viewer_ref = diff_viewer.clone();

    cx.subscribe(diff_viewer, {
        move |_this, _dv, event: &DiffViewerEvent, cx| {
            let file_path = diff_viewer_ref
                .read(cx)
                .file_path()
                .map(std::path::PathBuf::from);

            if let Some(path) = file_path {
                match event {
                    DiffViewerEvent::HunkStageRequested(hunk_idx) => {
                        let idx = *hunk_idx;
                        project.update(cx, |proj, cx| {
                            proj.stage_hunk(&path, idx, cx).detach();
                        });
                    }
                    DiffViewerEvent::HunkUnstageRequested(hunk_idx) => {
                        let idx = *hunk_idx;
                        project.update(cx, |proj, cx| {
                            proj.unstage_hunk(&path, idx, cx).detach();
                        });
                    }
                    DiffViewerEvent::LineStageRequested(line_pairs) => {
                        let pairs = line_pairs.clone();
                        project.update(cx, |proj, cx| {
                            proj.stage_lines(&path, &pairs, cx).detach();
                        });
                    }
                    DiffViewerEvent::LineUnstageRequested(line_pairs) => {
                        let pairs = line_pairs.clone();
                        project.update(cx, |proj, cx| {
                            proj.unstage_lines(&path, &pairs, cx).detach();
                        });
                    }
                }
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_commit_panel(
    cx: &mut Context<Workspace>,
    project: &Entity<GitProject>,
    ai: &Entity<AiGenerator>,
    commit_panel: &Entity<CommitPanel>,
) {
    let project = project.clone();
    let ai = ai.clone();
    let commit_panel_ref = commit_panel.clone();

    cx.subscribe(commit_panel, {
        move |this, _cp, event: &CommitPanelEvent, cx| match event {
            CommitPanelEvent::CommitRequested { message, amend } => {
                let previous_head_oid = project
                    .read(cx)
                    .recent_commits()
                    .first()
                    .map(|c| c.oid.to_string());
                let msg = message.clone();
                let is_amend = *amend;
                let worktree_path = this.effective_worktree_path(cx);
                project.update(cx, |proj, cx| {
                    proj.commit_at(&msg, is_amend, &worktree_path, cx).detach();
                });
                commit_panel_ref.update(cx, |cp, cx| {
                    cp.set_message(String::new(), cx);
                });
                if !is_amend {
                    if let Some(oid_hex) = previous_head_oid {
                        this.push_undo(
                            UndoEntry {
                                label: "Created commit".into(),
                                action: UndoAction::SoftResetHead(oid_hex),
                                created_at: Instant::now(),
                            },
                            cx,
                        );
                    }
                }
            }
            CommitPanelEvent::GenerateAiMessage => {
                commit_panel_ref.update(cx, |cp, cx| {
                    cp.set_ai_generating(true, cx);
                });

                let proj = project.read(cx);
                let repo_path = proj.repo_path().to_path_buf();
                let summary = proj.staged_summary();
                let ai_entity = ai.clone();
                let diff_repo_path = repo_path.clone();
                let settings_state = cx.global::<rgitui_settings::SettingsState>();
                let use_tools = settings_state.settings().ai.use_tools;
                cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                    let diff_text = cx
                        .background_executor()
                        .spawn(async move {
                            rgitui_git::compute_staged_diff_text(&diff_repo_path)
                                .unwrap_or_default()
                        })
                        .await;
                    cx.update(|cx| {
                        ai_entity.update(cx, |ai_gen, cx| {
                            ai_gen
                                .generate_commit_message_with_tools(
                                    diff_text, summary, repo_path, use_tools, cx,
                                )
                                .detach();
                        });
                    });
                })
                .detach();
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_toolbar(
    cx: &mut Context<Workspace>,
    project: &Entity<GitProject>,
    toolbar: &Entity<Toolbar>,
) {
    let _project = project.clone();

    cx.subscribe(toolbar, {
        move |this, _toolbar, event: &ToolbarEvent, cx| match event {
            ToolbarEvent::Fetch => {
                _project.update(cx, |proj, cx| {
                    proj.fetch_default(cx).detach();
                });
            }
            ToolbarEvent::Pull => {
                _project.update(cx, |proj, cx| {
                    proj.pull_default(cx).detach();
                });
            }
            ToolbarEvent::Push => {
                _project.update(cx, |proj, cx| {
                    proj.push_default(false, cx).detach();
                });
            }
            ToolbarEvent::StashSave => {
                _project.update(cx, |proj, cx| {
                    proj.stash_save(None, cx).detach();
                });
            }
            ToolbarEvent::StashPop => {
                _project.update(cx, |proj, cx| {
                    proj.stash_pop(0, cx).detach();
                });
            }
            ToolbarEvent::Branch => {
                this.dialogs.branch_dialog.update(cx, |bd, cx| {
                    bd.show_visible(None, cx);
                });
            }
            ToolbarEvent::Refresh => {
                _project.update(cx, |proj, cx| {
                    proj.refresh(cx).detach();
                });
            }
            ToolbarEvent::Settings => {
                this.open_or_focus_settings(cx);
            }
            ToolbarEvent::Search => {
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    tab.graph.update(cx, |g, cx| {
                        g.toggle_search(cx);
                    });
                }
            }
            ToolbarEvent::OpenFileExplorer => {
                let repo_path = this.effective_worktree_path(cx);
                super::layout::open_file_explorer(&repo_path);
            }
            ToolbarEvent::OpenTerminal => {
                let repo_path = this.effective_worktree_path(cx);
                let terminal_cmd = cx
                    .global::<rgitui_settings::SettingsState>()
                    .settings()
                    .terminal_command
                    .clone();
                super::layout::open_terminal(&repo_path, &terminal_cmd);
            }
            ToolbarEvent::OpenEditor => {
                let repo_path = this.effective_worktree_path(cx);
                let editor_cmd = cx
                    .global::<rgitui_settings::SettingsState>()
                    .settings()
                    .editor_command
                    .clone();
                super::layout::open_editor(&repo_path, &editor_cmd);
            }
            ToolbarEvent::CreatePr => {
                this.open_create_pr_dialog(cx);
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_blame_view(
    cx: &mut Context<Workspace>,
    blame_view: &Entity<BlameView>,
    graph: &Entity<GraphView>,
) {
    let graph = graph.clone();

    cx.subscribe(blame_view, {
        move |this, _bv, event: &BlameViewEvent, cx| match event {
            BlameViewEvent::CommitSelected(oid_str) => {
                if let Ok(oid) = git2::Oid::from_str(oid_str) {
                    graph.update(cx, |g, cx| {
                        g.scroll_to_commit(oid, cx);
                    });
                }
            }
            BlameViewEvent::Dismissed | BlameViewEvent::SwitchToDiff => {
                if let Some(tab) = this.tabs.get_mut(this.active_tab) {
                    tab.bottom_panel_mode = BottomPanelMode::Diff;
                    cx.notify();
                }
            }
            BlameViewEvent::SwitchToHistory => {
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    let tab = tab.clone();
                    this.execute_command(crate::CommandId::FileHistory, cx);
                    let _ = tab;
                }
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_file_history_view(
    cx: &mut Context<Workspace>,
    file_history_view: &Entity<FileHistoryView>,
    graph: &Entity<GraphView>,
) {
    let graph = graph.clone();

    cx.subscribe(file_history_view, {
        move |this, _fv, event: &FileHistoryViewEvent, cx| match event {
            FileHistoryViewEvent::CommitSelected(oid_str) => {
                if let Ok(oid) = git2::Oid::from_str(oid_str) {
                    graph.update(cx, |g, cx| {
                        g.scroll_to_commit(oid, cx);
                    });
                }
            }
            FileHistoryViewEvent::Dismissed | FileHistoryViewEvent::SwitchToDiff => {
                if let Some(tab) = this.tabs.get_mut(this.active_tab) {
                    tab.bottom_panel_mode = BottomPanelMode::Diff;
                    cx.notify();
                }
            }
            FileHistoryViewEvent::SwitchToBlame => {
                if let Some(tab) = this.tabs.get(this.active_tab) {
                    let tab = tab.clone();
                    this.execute_command(crate::CommandId::Blame, cx);
                    let _ = tab;
                }
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_reflog_view(
    cx: &mut Context<Workspace>,
    project: &Entity<GitProject>,
    reflog_view: &Entity<ReflogView>,
    graph: &Entity<GraphView>,
) {
    let graph = graph.clone();
    let project = project.clone();

    cx.subscribe(reflog_view, {
        move |this, _rv, event: &ReflogViewEvent, cx| match event {
            ReflogViewEvent::CommitSelected(oid_str) => {
                if let Ok(oid) = git2::Oid::from_str(oid_str) {
                    graph.update(cx, |g, cx| {
                        g.scroll_to_commit(oid, cx);
                    });
                }
            }
            ReflogViewEvent::Dismissed => {
                if let Some(tab) = this.tabs.get_mut(this.active_tab) {
                    tab.bottom_panel_mode = BottomPanelMode::Diff;
                    cx.notify();
                }
            }
            ReflogViewEvent::CheckoutCommit(oid) => {
                if let Ok(git_oid) = git2::Oid::from_str(oid) {
                    project.update(cx, |proj, cx| {
                        proj.checkout_commit(git_oid, cx).detach();
                    });
                }
            }
            ReflogViewEvent::CopyOID(oid) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(oid.clone()));
                let short = &oid[..7.min(oid.len())];
                this.show_toast(
                    format!("Copied OID: {}", short),
                    ToastKind::Success,
                    cx,
                );
            }
            ReflogViewEvent::ResetHard(oid) => {
                let short = &oid[..7.min(oid.len())];
                this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Reset (hard)",
                        format!(
                            "Hard reset the current branch to {}? All uncommitted changes and commits after this point will be lost.",
                            short
                        ),
                        ConfirmAction::ResetHard(oid.clone()),
                        cx,
                    );
                });
            }
            ReflogViewEvent::ResetSoft(oid) => {
                let short = &oid[..7.min(oid.len())];
                this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Reset (soft)",
                        format!(
                            "Soft reset the current branch to {}? Changes will be preserved in the index.",
                            short
                        ),
                        ConfirmAction::ResetSoft(oid.clone()),
                        cx,
                    );
                });
            }
            ReflogViewEvent::ResetMixed(oid) => {
                let short = &oid[..7.min(oid.len())];
                this.dialogs.confirm_dialog.update(cx, |cd, cx| {
                    cd.show_visible(
                        "Reset (mixed)",
                        format!(
                            "Mixed reset the current branch to {}? Changes will be unstaged.",
                            short
                        ),
                        ConfirmAction::ResetMixed(oid.clone()),
                        cx,
                    );
                });
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_submodule_view(
    cx: &mut Context<Workspace>,
    submodule_view: &Entity<SubmoduleView>,
    project: &Entity<GitProject>,
) {
    let project = project.clone();
    let submodule_view = submodule_view.clone();

    cx.subscribe(&submodule_view, {
        move |this, _sv, event: &SubmoduleViewEvent, cx| match event {
            SubmoduleViewEvent::InitSubmodule(name) => {
                let name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.submodule_init_async(name.clone(), cx).detach();
                });
            }
            SubmoduleViewEvent::UpdateSubmodule(name) => {
                let name = name.clone();
                project.update(cx, |proj, cx| {
                    proj.submodule_update_async(name.clone(), true, cx).detach();
                });
            }
            SubmoduleViewEvent::InitAll => {
                project.update(cx, |proj, cx| {
                    proj.submodule_init_all_async(cx).detach();
                });
            }
            SubmoduleViewEvent::UpdateAll => {
                project.update(cx, |proj, cx| {
                    proj.submodule_update_all_async(true, cx).detach();
                });
            }
            SubmoduleViewEvent::Dismissed => {
                if let Some(tab) = this.tabs.get_mut(this.active_tab) {
                    tab.bottom_panel_mode = BottomPanelMode::Diff;
                    cx.notify();
                }
            }
        }
    })
    .detach();
}

pub(super) fn subscribe_bisect_view(
    cx: &mut Context<Workspace>,
    project: &Entity<GitProject>,
    bisect_view: &Entity<BisectView>,
    graph: &Entity<GraphView>,
) {
    let graph = graph.clone();
    let project = project.clone();

    cx.subscribe(bisect_view, {
        move |this, _bv, event: &BisectViewEvent, cx| match event {
            BisectViewEvent::CommitSelected(oid_str) => {
                if let Ok(oid) = git2::Oid::from_str(oid_str) {
                    graph.update(cx, |g, cx| {
                        g.scroll_to_commit(oid, cx);
                    });
                }
            }
            BisectViewEvent::Dismissed => {
                if let Some(tab) = this.tabs.get_mut(this.active_tab) {
                    tab.bottom_panel_mode = BottomPanelMode::Diff;
                    cx.notify();
                }
            }
            BisectViewEvent::CopyOID(oid) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(oid.clone()));
                let short = &oid[..7.min(oid.len())];
                this.show_toast(format!("Copied OID: {}", short), ToastKind::Success, cx);
            }
            BisectViewEvent::Good(oid) => {
                if let Ok(git_oid) = git2::Oid::from_str(oid) {
                    project.update(cx, |proj, cx| {
                        proj.bisect_good(Some(git_oid), cx).detach();
                    });
                    this.show_toast(
                        format!("Marked {} as good", &oid[..7.min(oid.len())]),
                        ToastKind::Success,
                        cx,
                    );
                }
            }
            BisectViewEvent::Bad(oid) => {
                if let Ok(git_oid) = git2::Oid::from_str(oid) {
                    project.update(cx, |proj, cx| {
                        proj.bisect_bad(Some(git_oid), cx).detach();
                    });
                    this.show_toast(
                        format!("Marked {} as bad", &oid[..7.min(oid.len())]),
                        ToastKind::Success,
                        cx,
                    );
                }
            }
            BisectViewEvent::Skip(oid) => {
                if let Ok(git_oid) = git2::Oid::from_str(oid) {
                    project.update(cx, |proj, cx| {
                        proj.bisect_skip(Some(git_oid), cx).detach();
                    });
                    this.show_toast(
                        format!("Skipped {}", &oid[..7.min(oid.len())]),
                        ToastKind::Success,
                        cx,
                    );
                }
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::extract_stash_summary;

    #[test]
    fn test_extract_stash_summary_wip_with_subject() {
        let msg = "WIP on main: abc1234 last commit on main";
        assert_eq!(extract_stash_summary(msg), "last commit on main");
    }

    #[test]
    fn test_extract_stash_summary_on_with_subject() {
        // "On branch: <sha> <subject>" — real SHA (7 hex chars) followed by subject.
        let msg = "On feature: abc1234 WIP";
        assert_eq!(extract_stash_summary(msg), "WIP");
    }

    #[test]
    fn test_extract_stash_summary_no_subject_just_branch() {
        let msg = "On main: ghi9012";
        assert_eq!(extract_stash_summary(msg), "main");
    }

    #[test]
    fn test_extract_stash_summary_wip_no_subject() {
        let msg = "WIP on main: abc1234";
        assert_eq!(extract_stash_summary(msg), "main");
    }

    #[test]
    fn test_extract_stash_summary_custom_message() {
        let msg = "my important stash";
        assert_eq!(extract_stash_summary(msg), "my important stash");
    }

    #[test]
    fn test_extract_stash_summary_multiline() {
        let msg = "WIP on main: abc1234 my subject\n\nChanges:\n  file.txt";
        assert_eq!(extract_stash_summary(msg), "my subject");
    }

    #[test]
    fn test_extract_stash_summary_empty() {
        assert_eq!(extract_stash_summary(""), "Stash");
    }

    #[test]
    fn test_extract_stash_summary_whitespace_only() {
        assert_eq!(extract_stash_summary("   \n   "), "Stash");
    }

    #[test]
    fn test_extract_stash_summary_strips_sha_long_form() {
        // When after_colon starts with non-hex (e.g. "<sha><space><subject>" doesn't match
        // our pattern), fall back to using remaining as subject.
        let msg = "On main: abc1234d my subject";
        assert_eq!(extract_stash_summary(msg), "abc1234d my subject");
    }
}
