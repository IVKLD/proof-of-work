{ lib
, stdenv
, makeWrapper
, installShellFiles
, bash
, coreutils
, grim
}:

stdenv.mkDerivation {
  pname = "worktrack";
  version = "0.1.0";
  src = lib.cleanSource ./.;

  nativeBuildInputs = [
    makeWrapper
    installShellFiles
  ];

  installPhase = ''
    install -Dm755 bin/worktrack $out/bin/worktrack

    installShellCompletion --cmd worktrack \
      --bash <($out/bin/worktrack completions bash) \
      --zsh <($out/bin/worktrack completions zsh) \
      --fish <($out/bin/worktrack completions fish)

    wrapProgram $out/bin/worktrack \
      --prefix PATH : ${lib.makeBinPath [ bash coreutils grim ]}
  '';

  meta = with lib; {
    description = "Pure Wayland (niri) work time & window screenshot tracker (Prebuilt binary)";
    mainProgram = "worktrack";
    platforms = [ "x86_64-linux" ];
  };
}
