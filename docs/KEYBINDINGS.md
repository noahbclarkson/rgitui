# Keyboard shortcuts

<!-- Generated from the `commands!` declaration in `crates/rgitui_workspace/src/keymap/registry.rs`. Do not edit by hand. -->

Every shortcut below is rebindable. Create `keymap.json` next to `settings.json` in rgitui's config directory and bind the action names from the tables below.

`secondary` is the platform's primary modifier: `cmd` on macOS, `ctrl` everywhere else. Commands marked _unbound_ have no default keystroke and are reached from the command palette (`secondary-shift-p`).

## Customising

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

`docs/keymap.schema.json` lists every action name; point your editor at it for completion.

## Workspace

| Keystroke | Action | Description |
| --- | --- | --- |
| `secondary-shift-r` | `rgitui::Fetch` | Download objects and refs from the tracked remote. |
| _unbound_ | `rgitui::Pull` | Fetch from and integrate with the tracked remote branch. |
| _unbound_ | `rgitui::Push` | Update the remote ref along with associated objects. |
| _unbound_ | `rgitui::PushAll` | Push every open repository. |
| _unbound_ | `rgitui::PullAll` | Pull every open repository. |
| _unbound_ | `rgitui::ForcePush` | Overwrite the remote branch with the local one. |
| `secondary-enter` | `rgitui::Commit` | Commit the staged changes using the message in the commit panel. |
| `secondary-s` | `rgitui::StageAll` | Stage every change in the working tree. |
| `secondary-shift-s` or `secondary-u` | `rgitui::UnstageAll` | Unstage everything currently staged. |
| `secondary-z` | `rgitui::StashSave` | Stash the working tree and index. |
| `secondary-shift-z` | `rgitui::StashPop` | Apply the latest stash entry and drop it. |
| _unbound_ | `rgitui::StashApply` | Apply the latest stash entry and keep it. |
| _unbound_ | `rgitui::StashDrop` | Delete a stash entry. |
| `secondary-b` | `rgitui::CreateBranch` | Create a new branch. |
| _unbound_ | `rgitui::DeleteBranch` | Delete a branch. |
| _unbound_ | `rgitui::RenameBranch` | Rename a branch. |
| _unbound_ | `rgitui::MergeBranch` | Merge another branch into the current one. |
| _unbound_ | `rgitui::CreateTag` | Create a tag. |
| _unbound_ | `rgitui::CreateWorktree` | Create a linked worktree. |
| _unbound_ | `rgitui::CreatePr` | Open a pull request for the current branch. |
| _unbound_ | `rgitui::CherryPick` | Cherry-pick a commit onto the current branch. |
| _unbound_ | `rgitui::RevertCommit` | Revert a commit. |
| _unbound_ | `rgitui::InteractiveRebase` | Start an interactive rebase. |
| _unbound_ | `rgitui::DiscardAll` | Discard every uncommitted change. |
| _unbound_ | `rgitui::CleanUntracked` | Delete untracked files. |
| _unbound_ | `rgitui::ResetHard` | Reset the working tree and index to HEAD. |
| _unbound_ | `rgitui::AbortOperation` | Abort the merge, rebase, cherry-pick or revert in progress. |
| _unbound_ | `rgitui::ContinueMerge` | Continue the merge, rebase, cherry-pick or revert in progress. |
| _unbound_ | `rgitui::ToggleDiffMode` | Switch the diff viewer between unified and side-by-side. |
| _unbound_ | `rgitui::Search` | Search the commit graph. |
| `secondary-g` | `rgitui::AiMessage` | Generate a commit message with the configured AI provider. |
| `f5` | `rgitui::Refresh` | Reload the repository state from disk. |
| `secondary-,` | `rgitui::Settings` | Open the settings window. |
| `secondary-o` | `rgitui::OpenRepo` | Open the repository picker. |
| `secondary-h` | `rgitui::WorkspaceHome` | Close every tab and return to the workspace home screen. |
| _unbound_ | `rgitui::RestoreLastWorkspace` | Reopen the most recently saved workspace. |
| `?` | `rgitui::Shortcuts` | Show the keyboard shortcut reference. |
| `secondary-shift-b` | `rgitui::SwitchBranch` | Focus the sidebar to switch branches. |
| _unbound_ | `rgitui::Blame` | Blame the selected file. |
| _unbound_ | `rgitui::Undo` | Undo the last git operation. |
| _unbound_ | `rgitui::FileHistory` | Show the commit history of the selected file. |
| _unbound_ | `rgitui::Reflog` | Show the reflog. |
| _unbound_ | `rgitui::Submodules` | Show the submodule list. |
| _unbound_ | `rgitui::Bisect` | Show the bisect log. |
| _unbound_ | `rgitui::BisectStart` | Start a bisect session. |
| _unbound_ | `rgitui::BisectGood` | Mark the current bisect commit as good. |
| _unbound_ | `rgitui::BisectBad` | Mark the current bisect commit as bad. |
| _unbound_ | `rgitui::BisectReset` | End the bisect session and restore HEAD. |
| _unbound_ | `rgitui::BisectSkip` | Skip the current bisect commit. |
| _unbound_ | `rgitui::GlobalSearch` | Search the contents of the working tree. |
| `alt-5` | `rgitui::ToggleIssues` | Toggle the issues panel. |
| `alt-6` | `rgitui::TogglePullRequests` | Toggle the pull requests panel. |
| `alt-7` | `rgitui::ToggleBranchHealth` | Toggle the branch health panel. |
| `alt-8` | `rgitui::ToggleStashes` | Toggle the stashes panel. |
| _unbound_ | `rgitui::StashBranch` | Create a branch from a stash entry. |
| `secondary-shift-t` or `alt-9` | `rgitui::OpenThemeEditor` | Open the theme editor. |
| `secondary-shift-p` | `rgitui::CommandPalette` | Toggle the command palette. |
| `secondary-tab` | `rgitui::NextTab` | Activate the next repository tab. |
| `secondary-shift-tab` | `rgitui::PrevTab` | Activate the previous repository tab. |
| `secondary-w` | `rgitui::CloseTab` | Close the active repository tab. |

## Key contexts

| Context | Set on |
| --- | --- |
| `Workspace` | the workspace root, so it is always in scope |
| `modal` | added to the workspace root while any overlay or dialog is open |
| `TextInput` | any focused text field, so single-key shortcuts do not steal typing |

Contexts combine with `&&`, `||` and `!`, and `>` matches a descendant.
