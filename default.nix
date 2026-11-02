{ lib
, rustPlatform
, pkg-config
, wayland
, makeWrapper
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
  ];

  buildInputs = [
    wayland
  ];

  postInstall = ''
    wrapProgram $out/bin/worktrack \
      --prefix PATH : ${lib.makeBinPath [ bash coreutils ]}
  '';

  meta = with lib; {
    description = "Pure Wayland (niri) work time & window screenshot tracker with SCTK indicator";
    mainProgram = "worktrack";
    platforms = platforms.linux;
  };
}
