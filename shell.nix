{ pkgs ? import <nixpkgs> {} }:
pkgs.mkShell {
  packages = with pkgs; [
    cargo
    rustc
    rustfmt
    clippy
    pkg-config
    wayland
    grim
  ];
  shellHook = ''
    export PATH="$PWD/target/release:$PWD/target/debug:$PATH"
  '';
}
