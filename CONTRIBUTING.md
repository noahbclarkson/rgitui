# Contributing to rgitui

Thank you for your interest in contributing to rgitui! This document provides guidelines for contributing.

## Prerequisites

- Rust stable toolchain (see `rust-toolchain.toml`)
- A local checkout of [Zed](https://github.com/zed-industries/zed) at `../zed` (sibling directory)
- System dependencies for GPUI (see below)

### System Dependencies

**Linux:**
```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential \
  cmake \
  clang \
  lld \
  gcc \
  g++ \
  pkg-config \
  libasound2-dev \
  libfontconfig-dev \
  libfreetype-dev \
  libgit2-dev \
  libglib2.0-dev \
  libssl-dev \
  libsqlite3-dev \
  libva-dev \
  libvulkan1 \
  libvulkan-dev \
  libwayland-dev \
  libx11-xcb-dev \
  libxcb1-dev \
  libxkbcommon-x11-dev \
  libxkbcommon-dev \
  libxcomposite-dev \
  libxdamage-dev \
  libxext-dev \
  libxfixes-dev \
  libxrandr-dev \
  libxi-dev \
  libxcursor-dev \
  libdrm-dev \
  libgbm-dev \
  libzstd-dev \
  vulkan-tools
```

**Windows:**
- Visual Studio Build Tools with C++ workload

**macOS:**
- Xcode Command Line Tools

## Building

```bash
# Clone both repos
git clone https://github.com/mrnoa/rgitui.git
git clone https://github.com/zed-industries/zed.git

# Build
cd rgitui
cargo build

# Run
cargo run
```

## Development Workflow

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Make your changes
4. Run checks:
   ```bash
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
5. Commit your changes
6. Push to your fork and open a Pull Request

## Code Style

- Follow standard Rust conventions
- Run `cargo fmt` before committing
- Ensure `cargo clippy` passes without warnings
- Write descriptive commit messages

## Architecture

rgitui is organized as a Cargo workspace with the following crates:

| Crate | Description |
|-------|-------------|
| `rgitui` | Main binary entry point |
| `rgitui_workspace` | Root workspace view, panels, dialogs, and UI orchestration |
| `rgitui_git` | Git operations via libgit2, repository management |
| `rgitui_graph` | Commit graph visualization and layout |
| `rgitui_diff` | Diff viewer (unified and side-by-side) |
| `rgitui_ui` | Reusable UI components (buttons, labels, modals, etc.) |
| `rgitui_theme` | Theme system with semantic colors |
| `rgitui_settings` | Settings persistence, keychain integration |
| `rgitui_ai` | AI commit message generation |

## Reporting Bugs

Please use the GitHub issue template for bug reports. Include:
- Steps to reproduce
- Expected vs actual behavior
- Your environment (OS, version)
