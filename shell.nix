{ pkgs ? import <nixpkgs> {} }:

let
  rustOverlay = builtins.fetchTarball "https://github.com/oxalica/rust-overlay/archive/master.tar.gz";
  pkgsWithRust = import <nixpkgs> {
    overlays = [ (import rustOverlay) ];
  };
  rust = pkgsWithRust.rust-bin.stable.latest.default;
in
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    pkg-config
    cmake
    perl
    rust
    patchelf
  ];

  buildInputs = with pkgs; [
    # GPUI dependencies
    fontconfig
    freetype
    libxkbcommon
    wayland
    libglvnd
    vulkan-loader
    openssl
    zlib
    alsa-lib
    libgit2
    libdrm
    libgbm
    xorg.libxcb
    xorg.libX11
    xorg.libXi
    xorg.libXrandr
    xorg.libXcursor
    libxcomposite
    libxdamage
    libxext
    libxfixes
    libxrandr
    sqlite
  ];

  LD_LIBRARY_PATH = with pkgs; lib.makeLibraryPath [
    wayland
    libglvnd
    vulkan-loader
    libxkbcommon
    fontconfig
    freetype
    xorg.libxcb
    xorg.libX11
    libdrm
    libgbm
  ];
}
