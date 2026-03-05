{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    pkg-config
    cmake
    perl
    # Rust toolchain comes from the system (NixOS config), not nix-shell
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
