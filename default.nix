{ lib
, rustPlatform
, pkg-config
, wayland
, makeWrapper
, installShellFiles
, bash
, coreutils
}:

rustPlatform.buildRustPackage {
  pname = "worktrack";
  version = "0.1.0";
  src = lib.cleanSource ./.;

  cargoLock = {
    lockFile = ./Cargo.lock;
  };

  nativeBuildInputs = [
    pkg-config
    makeWrapper
    installShellFiles
  ];

  buildInputs = [
    wayland
  ];

  postInstall = ''
    installShellCompletion --cmd worktrack \
      --bash <($out/bin/worktrack completions bash) \
      --zsh <($out/bin/worktrack completions zsh) \
      --fish <($out/bin/worktrack completions fish)

    wrapProgram $out/bin/worktrack \
      --prefix PATH : ${lib.makeBinPath [ bash coreutils ]}
  '';

  meta = with lib; {
    description = "Pure Wayland (niri) work time & window screenshot tracker with SCTK indicator";
    mainProgram = "worktrack";
    platforms = platforms.linux;
  };
}
