{ lib
, stdenv
, fetchurl
, makeWrapper
, installShellFiles
, grim
}:

let
  version = "0.1.1";
in
stdenv.mkDerivation {
  pname = "worktrack";
  inherit version;

  src = fetchurl {
    url = "https://github.com/IVKLD/proof-of-work/releases/download/v${version}/worktrack-linux-x86_64";
    hash = "sha256-UfTo/ZseuUrPBoAN9E/hKMPlB2vqYDP8b2oACPNxhzk=";
  };

  dontUnpack = true;

  nativeBuildInputs = [
    makeWrapper
    installShellFiles
  ];

  installPhase = ''
    install -Dm755 $src $out/bin/worktrack

    installShellCompletion --cmd worktrack \
      --bash <($out/bin/worktrack completions bash) \
      --zsh <($out/bin/worktrack completions zsh) \
      --fish <($out/bin/worktrack completions fish)

    wrapProgram $out/bin/worktrack \
      --prefix PATH : ${lib.makeBinPath [ grim ]}
  '';

  meta = with lib; {
    description = "Pure Wayland (niri) work time & window screenshot tracker (Prebuilt binary)";
    mainProgram = "worktrack";
    platforms = [ "x86_64-linux" ];
  };
}
