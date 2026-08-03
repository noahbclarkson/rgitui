# Keyboard shortcuts

<!-- Generated from the `commands!` declaration in `crates/rgitui_workspace/src/keymap/registry.rs`. Do not edit by hand. -->

Every shortcut below is rebindable, and press `?` in rgitui to see the ones actually in force — that reference is generated from the same declaration as this page, so it follows your own keybindings rather than the defaults.

`secondary` is the platform's primary modifier: `cmd` on macOS, `ctrl` everywhere else. Commands marked _unbound_ have no default keystroke and are reached from the command palette (`secondary-shift-p`).

## Customising

Keybindings live in `keymap.json`, next to `settings.json` in rgitui's config directory:

| Platform | Path |
| --- | --- |
| Linux | `~/.config/rgitui/keymap.json` |
| macOS | `~/Library/Application Support/rgitui/keymap.json` |
| Windows | `%APPDATA%\rgitui\keymap.json` |

Run the **Open keymap.json** command from the palette, or use the button in the shortcut reference, to create the file with a commented example already in it and open it in your editor.

```jsonc
[
  {
    "context": "Workspace && !modal",
    "bindings": {
      // Rebind staging.
      "ctrl-alt-s": "rgitui::StageAll",
      // Remove a default binding.
      "secondary-s": null
    }
  }
]
```

The file is reloaded when you save it. Bindings you add win over the defaults. Two bindings on the same keystroke in overlapping contexts, or a binding that shadows the prefix of a chord, are reported as a toast and the losing binding is dropped rather than silently ignored.

`docs/keymap.schema.json` lists every action name with its description; associate it with `keymap.json` in your editor's JSON schema settings for completion and hovers.

## Workspace

| Keystroke | Context | Action | Description |
| --- | --- | --- | --- |
| `secondary-shift-r` | `Workspace && !modal` | `rgitui::Fetch` | Download objects and refs from the tracked remote. |
| _unbound_ | — | `rgitui::Pull` | Fetch from and integrate with the tracked remote branch. |
| _unbound_ | — | `rgitui::Push` | Update the remote ref along with associated objects. |
| _unbound_ | — | `rgitui::PushAll` | Push every open repository. |
| _unbound_ | — | `rgitui::PullAll` | Pull every open repository. |
| _unbound_ | — | `rgitui::ForcePush` | Overwrite the remote branch with the local one. |
| `secondary-enter` | `Workspace && !modal` | `rgitui::Commit` | Commit the staged changes using the message in the commit panel. |
| `secondary-s` | `Workspace && !modal` | `rgitui::StageAll` | Stage every change in the working tree. |
| `secondary-shift-s` or `secondary-u` | `Workspace && !modal` | `rgitui::UnstageAll` | Unstage everything currently staged. |
| `secondary-z` | `Workspace && !modal` | `rgitui::StashSave` | Stash the working tree and index. |
| `secondary-shift-z` | `Workspace && !modal` | `rgitui::StashPop` | Apply the latest stash entry and drop it. |
| _unbound_ | — | `rgitui::StashApply` | Apply the latest stash entry and keep it. |
| _unbound_ | — | `rgitui::StashDrop` | Delete a stash entry. |
| `secondary-b` | `Workspace && !modal` | `rgitui::CreateBranch` | Create a new branch. |
| _unbound_ | — | `rgitui::DeleteBranch` | Delete a branch. |
| _unbound_ | — | `rgitui::RenameBranch` | Rename a branch. |
| _unbound_ | — | `rgitui::MergeBranch` | Merge another branch into the current one. |
| _unbound_ | — | `rgitui::CreateTag` | Create a tag. |
| _unbound_ | — | `rgitui::CreateWorktree` | Create a linked worktree. |
| _unbound_ | — | `rgitui::CreatePr` | Open a pull request for the current branch. |
| _unbound_ | — | `rgitui::CherryPick` | Cherry-pick a commit onto the current branch. |
| _unbound_ | — | `rgitui::RevertCommit` | Revert a commit. |
| _unbound_ | — | `rgitui::InteractiveRebase` | Start an interactive rebase. |
| _unbound_ | — | `rgitui::DiscardAll` | Discard every uncommitted change. |
| _unbound_ | — | `rgitui::CleanUntracked` | Delete untracked files. |
| _unbound_ | — | `rgitui::ResetHard` | Reset the working tree and index to HEAD. |
| _unbound_ | — | `rgitui::AbortOperation` | Abort the merge, rebase, cherry-pick or revert in progress. |
| _unbound_ | — | `rgitui::ContinueMerge` | Continue the merge, rebase, cherry-pick or revert in progress. |
| `shift-d` | `Workspace && !modal && !TextInput` | `rgitui::ToggleDiffMode` | Switch the diff viewer between unified and side-by-side. |
| `secondary-f` or `/` | `Workspace && !modal` or `Workspace && !modal && !TextInput` | `rgitui::Search` | Search the commit graph. |
| `secondary-g` | `Workspace && !modal` | `rgitui::AiMessage` | Generate a commit message with the configured AI provider. |
| `f5` | `Workspace && !modal` | `rgitui::Refresh` | Reload the repository state from disk. |
| `secondary-,` | `Workspace` | `rgitui::Settings` | Open the settings window. |
| `secondary-o` | `Workspace` | `rgitui::OpenRepo` | Open the repository picker. |
| `ctrl-h` | `Workspace && !modal` | `rgitui::WorkspaceHome` | Close every tab and return to the workspace home screen. |
| _unbound_ | — | `rgitui::RestoreLastWorkspace` | Reopen the most recently saved workspace. |
| `?` | `Workspace && !TextInput` | `rgitui::Shortcuts` | Show the keyboard shortcut reference. |
| `secondary-shift-b` | `Workspace && !modal` | `rgitui::SwitchBranch` | Focus the sidebar to switch branches. |
| _unbound_ | — | `rgitui::Blame` | Blame the selected file. |
| _unbound_ | — | `rgitui::Undo` | Undo the last git operation. |
| _unbound_ | — | `rgitui::FileHistory` | Show the commit history of the selected file. |
| _unbound_ | — | `rgitui::Reflog` | Show the reflog. |
| _unbound_ | — | `rgitui::Submodules` | Show the submodule list. |
| _unbound_ | — | `rgitui::Bisect` | Show the bisect log. |
| _unbound_ | — | `rgitui::BisectStart` | Start a bisect session. |
| _unbound_ | — | `rgitui::BisectGood` | Mark the current bisect commit as good. |
| _unbound_ | — | `rgitui::BisectBad` | Mark the current bisect commit as bad. |
| _unbound_ | — | `rgitui::BisectReset` | End the bisect session and restore HEAD. |
| _unbound_ | — | `rgitui::BisectSkip` | Skip the current bisect commit. |
| `secondary-shift-f` | `Workspace && !modal` | `rgitui::GlobalSearch` | Search the contents of the working tree. |
| `alt-5` | `Workspace && !modal` | `rgitui::ToggleIssues` | Toggle the issues panel. |
| `alt-6` | `Workspace && !modal` | `rgitui::TogglePullRequests` | Toggle the pull requests panel. |
| `alt-7` | `Workspace && !modal` | `rgitui::ToggleBranchHealth` | Toggle the branch health panel. |
| `alt-8` | `Workspace && !modal` | `rgitui::ToggleStashes` | Toggle the stashes panel. |
| _unbound_ | — | `rgitui::StashBranch` | Create a branch from a stash entry. |
| `secondary-shift-t` or `alt-9` | `Workspace` | `rgitui::OpenThemeEditor` | Open the theme editor. |
| `secondary-shift-p` | `Workspace` | `rgitui::CommandPalette` | Toggle the command palette. |
| `ctrl-tab` | `Workspace && !modal` | `rgitui::NextTab` | Activate the next repository tab. |
| `ctrl-shift-tab` | `Workspace && !modal` | `rgitui::PrevTab` | Activate the previous repository tab. |
| `secondary-w` | `Workspace && !modal` | `rgitui::CloseTab` | Close the active repository tab. |
| `alt-1` | `Workspace && !modal` | `rgitui::FocusSidebar` | Move keyboard focus to the sidebar. |
| `alt-2` | `Workspace && !modal` | `rgitui::FocusGraph` | Move keyboard focus to the commit graph. |
| `alt-3` | `Workspace && !modal` | `rgitui::FocusDetailPanel` | Move keyboard focus to the commit detail panel. |
| `alt-4` | `Workspace && !modal` | `rgitui::FocusDiffViewer` | Move keyboard focus to the diff viewer. |
| `tab` | `Workspace && !modal && !TextInput` | `rgitui::FocusNextPanel` | Move keyboard focus to the next panel. |
| `shift-tab` | `Workspace && !modal && !TextInput` | `rgitui::FocusPrevPanel` | Move keyboard focus to the previous panel. |
| `secondary-[` | `Workspace && !modal` | `rgitui::ShrinkDetailPanel` | Narrow the detail panel. |
| `secondary-]` | `Workspace && !modal` | `rgitui::GrowDetailPanel` | Widen the detail panel. |
| `secondary-up` | `Workspace && !modal` | `rgitui::ShrinkDiffViewer` | Shorten the diff viewer. |
| `secondary-down` | `Workspace && !modal` | `rgitui::GrowDiffViewer` | Heighten the diff viewer. |
| _unbound_ | — | `rgitui::OpenKeymap` | Open keymap.json in your editor to rebind shortcuts. |

## Menu

| Keystroke | Context | Action | Description |
| --- | --- | --- | --- |
| `escape` | `Workspace \|\| SettingsWindow` | `menu::Cancel` | Dismiss the focused overlay, dialog, search or selection. |
| `enter` or `space` | `Workspace \|\| SettingsWindow` or `List && !TextInput` | `menu::Confirm` | Activate the selected row, or submit the focused dialog. |
| `down` or `j` | `List` or `List && !TextInput` | `menu::SelectNext` | Move the selection down one row. |
| `up` or `k` | `List` or `List && !TextInput` | `menu::SelectPrev` | Move the selection up one row. |
| `home` or `g` | `List` or `List && !TextInput` | `menu::SelectFirst` | Move the selection to the first row. |
| `end` or `shift-g` | `List` or `List && !TextInput` | `menu::SelectLast` | Move the selection to the last row. |

## GraphView

| Keystroke | Context | Action | Description |
| --- | --- | --- | --- |
| `down` or `j` | `GraphView && !modal` or `GraphView && !modal && !TextInput` | `graph::GraphSelectNext` | Select the next commit in the graph. |
| `up` or `k` | `GraphView && !modal` or `GraphView && !modal && !TextInput` | `graph::GraphSelectPrev` | Select the previous commit in the graph. |
| `home` or `g` | `GraphView && !modal` or `GraphView && !modal && !TextInput` | `graph::GraphSelectFirst` | Select the newest commit in the graph. |
| `end` or `shift-g` | `GraphView && !modal` or `GraphView && !modal && !TextInput` | `graph::GraphSelectLast` | Select the oldest loaded commit in the graph. |
| `shift-down` or `shift-j` | `GraphView && !modal && !TextInput` | `graph::GraphExtendSelectionNext` | Add the next commit in the graph to the selection. |
| `shift-up` or `shift-k` | `GraphView && !modal && !TextInput` | `graph::GraphExtendSelectionPrev` | Add the previous commit in the graph to the selection. |
| `s` | `GraphView && !modal && !TextInput` | `graph::SquashSelected` | Squash the selected commits into the oldest of them. |
| `escape` | `GraphView && !modal` | `graph::GraphCancel` | Close the graph search, or dismiss the graph context menu. |
| `y` | `GraphView && !modal && !TextInput` | `graph::CopyCommitSha` | Copy the selected commit's SHA to the clipboard. |
| `shift-c` | `GraphView && !modal && !TextInput` | `graph::CopyCommitMessage` | Copy the selected commit's message to the clipboard. |

## DiffViewer

| Keystroke | Context | Action | Description |
| --- | --- | --- | --- |
| `down` or `j` | `DiffViewer && !modal && !TextInput` | `diff::DiffSelectNext` | Move the diff cursor down one row. |
| `up` or `k` | `DiffViewer && !modal && !TextInput` | `diff::DiffSelectPrev` | Move the diff cursor up one row. |
| `home` or `g` | `DiffViewer && !modal && !TextInput` | `diff::DiffSelectFirst` | Move the diff cursor to the first row. |
| `end` or `shift-g` | `DiffViewer && !modal && !TextInput` | `diff::DiffSelectLast` | Move the diff cursor to the last row. |
| `]` | `DiffViewer && !modal && !TextInput` | `diff::NextHunk` | Jump to the next hunk. |
| `[` | `DiffViewer && !modal && !TextInput` | `diff::PrevHunk` | Jump to the previous hunk. |
| `d` | `DiffViewer && !modal && !TextInput` | `diff::ToggleDiffDisplayMode` | Cycle the diff viewer's display mode. |
| `p` | `DiffViewer && !modal && !TextInput` | `diff::TogglePartialSelection` | Toggle line-level selection in the diff viewer. |
| `s` or `shift-s` | `DiffViewer && !modal && !TextInput` | `diff::StageSelection` | Stage the hunks or lines under the diff selection. |
| `u` or `shift-u` | `DiffViewer && !modal && !TextInput` | `diff::UnstageSelection` | Unstage the hunks or lines under the diff selection. |
| `alt-s` | `DiffViewer && !modal && !TextInput` | `diff::StageCurrentHunk` | Stage the hunk under the diff cursor. |
| `alt-u` | `DiffViewer && !modal && !TextInput` | `diff::UnstageCurrentHunk` | Unstage the hunk under the diff cursor. |
| `secondary-c` | `DiffViewer && !modal && !TextInput` | `diff::CopyDiffSelection` | Copy the selected diff lines to the clipboard. |
| `secondary-a` | `DiffViewer && !modal && !TextInput` | `diff::SelectAllDiffLines` | Select every line in the diff. |

## DetailPanel

| Keystroke | Context | Action | Description |
| --- | --- | --- | --- |
| `v` | `DetailPanel && !modal && !TextInput` | `detail::ToggleFileTree` | Switch the changed-files list between the flat and tree layouts. |
| `[` | `DetailPanel && !modal && !TextInput` | `detail::PrevCommitDetails` | Show the previous commit's details. |
| `]` | `DetailPanel && !modal && !TextInput` | `detail::NextCommitDetails` | Show the next commit's details. |
| `/` or `secondary-f` | `DetailPanel && !modal && !TextInput` or `DetailPanel && !modal` | `detail::FileSearch` | Filter the changed-files list. |

## Sidebar

| Keystroke | Context | Action | Description |
| --- | --- | --- | --- |
| `s` | `Sidebar && !modal && !TextInput` | `sidebar::ToggleStageRow` | Stage or unstage the selected file. |
| `x` or `delete` | `Sidebar && !modal && !TextInput` or `Sidebar && !modal` | `sidebar::DiscardRow` | Discard the selected change, or delete the selected branch, tag or stash. |
| `/` or `secondary-f` | `Sidebar && !modal && !TextInput` or `Sidebar && !modal` | `sidebar::FilterBranches` | Filter the branch list. |

## BlameView

| Keystroke | Context | Action | Description |
| --- | --- | --- | --- |
| `escape` or `d` | `BlameView && !modal` or `BlameView && !modal && !TextInput` | `blame::BlameShowDiff` | Leave the blame view and go back to the diff. |
| `h` | `BlameView && !modal && !TextInput` | `blame::BlameShowHistory` | Show the blamed file's commit history. |

## FileHistoryView

| Keystroke | Context | Action | Description |
| --- | --- | --- | --- |
| `escape` or `d` | `FileHistoryView && !modal` or `FileHistoryView && !modal && !TextInput` | `history::HistoryShowDiff` | Leave the file history and go back to the diff. |
| `b` | `FileHistoryView && !modal && !TextInput` | `history::HistoryShowBlame` | Blame the file whose history is shown. |

## InteractiveRebase

| Keystroke | Context | Action | Description |
| --- | --- | --- | --- |
| `secondary-up` | `InteractiveRebase` | `rebase::RebaseMoveUp` | Move the selected commit earlier in the rebase plan. |
| `secondary-down` | `InteractiveRebase` | `rebase::RebaseMoveDown` | Move the selected commit later in the rebase plan. |
| `p` | `InteractiveRebase && !TextInput` | `rebase::RebasePick` | Keep the selected commit as it is. |
| `r` | `InteractiveRebase && !TextInput` | `rebase::RebaseReword` | Reword the selected commit's message. |
| `s` | `InteractiveRebase && !TextInput` | `rebase::RebaseSquash` | Squash the selected commit into the previous one. |
| `f` | `InteractiveRebase && !TextInput` | `rebase::RebaseFixup` | Squash the selected commit into the previous one, discarding its message. |
| `d` | `InteractiveRebase && !TextInput` | `rebase::RebaseDrop` | Drop the selected commit. |

## ThemeEditor

| Keystroke | Context | Action | Description |
| --- | --- | --- | --- |
| `tab` | `ThemeEditor` | `theme::ThemeEditorNextField` | Focus the next field in the theme editor. |
| `shift-tab` | `ThemeEditor` | `theme::ThemeEditorPrevField` | Focus the previous field in the theme editor. |

## CreatePrDialog

| Keystroke | Context | Action | Description |
| --- | --- | --- | --- |
| `shift-enter` | `CreatePrDialog` | `pr::SubmitPullRequest` | Open the pull request described in the dialog. |

## Key contexts

A binding fires when its context matches somewhere on the path from the focused element to the window root, and the deepest matching binding wins. That is what lets one keystroke mean different things in different panels without any `if focused` checks.

| Context | Set on |
| --- | --- |
| `Workspace` | the workspace root, so it is always in scope |
| `SettingsWindow` | the settings window root |
| `modal` | added to the workspace root while any overlay or dialog is open |
| `TextInput` | any text field, so single-key shortcuts do not steal typing |
| `List` | every panel, picker and dialog that owns a row selection |
| a view name | the panel, overlay or dialog of that name — see the tables above |

Contexts combine with `&&`, `||` and `!`, and `>` matches a descendant. `!TextInput` is false whenever a text field is anywhere on the focus path, which is why the vim-style letters carry it and the arrow keys do not.

## Where a panel wins a keystroke

The deepest match wins, so a few of the shortcuts above cannot be reached while a particular panel has focus. That is intended — the alternative would be a panel unable to give a letter its own meaning. Both bindings stay active; only one of them is what the keystroke does in that panel.

| Keystroke | Runs | While focused | So this is out of reach |
| --- | --- | --- | --- |
| `/` | `sidebar::FilterBranches` | the sidebar | `rgitui::Search` |
| `secondary-f` | `sidebar::FilterBranches` | the sidebar | `rgitui::Search` |
| `escape` | `graph::GraphCancel` | the commit graph | `menu::Cancel` |
| `/` | `detail::FileSearch` | the commit detail panel | `rgitui::Search` |
| `secondary-f` | `detail::FileSearch` | the commit detail panel | `rgitui::Search` |
| `escape` | `blame::BlameShowDiff` | the blame view | `menu::Cancel` |
| `escape` | `history::HistoryShowDiff` | the file history | `menu::Cancel` |
