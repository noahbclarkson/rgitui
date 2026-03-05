# rgitui

A GPU-accelerated Git GUI built with [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) (Zed's UI framework) and Rust.

## Features

- **Commit graph** — virtualized, scrollable commit history with lane-based coloring and GitKraken-style angular curves
- **Diff viewer** — unified and side-by-side modes with virtualized rendering, hunk-level staging/unstaging
- **Working tree** — stage, unstage, and discard changes per-file or in bulk
- **Branch management** — checkout, create branches, view local and remote branches
- **Stash support** — save and pop stashes from the toolbar
- **Remote operations** — fetch, pull, push
- **AI commit messages** — optional Gemini API integration for generating commit messages
- **Resizable panels** — drag-resize sidebar, detail panel, and diff viewer
- **Theming** — Catppuccin Mocha (default), Catppuccin Latte, One Dark

## Architecture

Cargo workspace with 9 crates:

| Crate | Purpose |
|---|---|
| `rgitui` | Binary entry point |
| `rgitui_workspace` | App shell (Workspace, Sidebar, Toolbar, etc.) |
| `rgitui_graph` | Commit graph rendering with `uniform_list` |
| `rgitui_diff` | Virtualized diff viewer |
| `rgitui_git` | Git operations via `git2`, graph layout algorithm |
| `rgitui_ui` | UI component library (Button, Label, Badge, etc.) |
| `rgitui_theme` | Theme system with Catppuccin + One Dark palettes |
| `rgitui_ai` | AI commit message generation (Gemini API) |
| `rgitui_settings` | Application settings (JSON config) |

## Prerequisites

- Rust toolchain (stable)
- GPUI dependencies — requires a local checkout of [Zed](https://github.com/zed-industries/zed) at `../zed/` relative to this project
- Linux: Wayland or X11 display server
- System libs: `pkg-config`, `openssl-dev`, `libgit2` (for `git2` crate)

## Building

```bash
cargo build
cargo run
```

To open a specific repository:

```bash
cargo run -- /path/to/repo
```

## Keyboard Shortcuts

| Key | Action |
|---|---|
| `j` / `k` | Navigate commits in graph |
| `Ctrl+[` / `Ctrl+]` | Resize detail panel |
| `Ctrl+Up` / `Ctrl+Down` | Resize diff viewer |
| `Ctrl+P` | Open command palette |

## License

MIT
