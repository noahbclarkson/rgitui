#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

echo "Pulling latest..."
git pull --ff-only 2>/dev/null || echo "Not a git remote or already up to date"

echo "Building rgitui (release)..."
nix-shell --run "cargo build --release 2>&1"

echo "Patchelfing binary..."
nix-shell --run '
RPATH=$(patchelf --print-rpath target/release/rgitui)
EXTRA_RPATH=$(nix-build "<nixpkgs>" -A wayland --no-out-link)/lib
EXTRA_RPATH="$EXTRA_RPATH:$(nix-build "<nixpkgs>" -A libglvnd --no-out-link)/lib"
EXTRA_RPATH="$EXTRA_RPATH:$(nix-build "<nixpkgs>" -A vulkan-loader --no-out-link)/lib"
EXTRA_RPATH="$EXTRA_RPATH:$(nix-build "<nixpkgs>" -A libxkbcommon --no-out-link)/lib"
EXTRA_RPATH="$EXTRA_RPATH:$(nix-build "<nixpkgs>" -A fontconfig.lib --no-out-link)/lib"
EXTRA_RPATH="$EXTRA_RPATH:$(nix-build "<nixpkgs>" -A freetype --no-out-link)/lib"
EXTRA_RPATH="$EXTRA_RPATH:$(nix-build "<nixpkgs>" -A xorg.libxcb --no-out-link)/lib"
EXTRA_RPATH="$EXTRA_RPATH:$(nix-build "<nixpkgs>" -A xorg.libX11 --no-out-link)/lib"
EXTRA_RPATH="$EXTRA_RPATH:$(nix-build "<nixpkgs>" -A libdrm --no-out-link)/lib"
EXTRA_RPATH="$EXTRA_RPATH:$(nix-build "<nixpkgs>" -A libgbm --no-out-link)/lib"
EXTRA_RPATH="$EXTRA_RPATH:$(nix-build "<nixpkgs>" -A openssl.out --no-out-link)/lib"
EXTRA_RPATH="$EXTRA_RPATH:$(nix-build "<nixpkgs>" -A libgit2 --no-out-link)/lib"
EXTRA_RPATH="$EXTRA_RPATH:$(nix-build "<nixpkgs>" -A alsa-lib --no-out-link)/lib"
EXTRA_RPATH="$EXTRA_RPATH:$(nix-build "<nixpkgs>" -A zlib --no-out-link)/lib"
patchelf --set-rpath "$RPATH:$EXTRA_RPATH" target/release/rgitui
'

mkdir -p ~/.local/bin
ln -sf "$(pwd)/target/release/rgitui" ~/.local/bin/rgitui

echo "Done! rgitui installed to ~/.local/bin/rgitui"
