{ lib
, rustPlatform
, pkg-config
, makeWrapper
, xz
, pixz
, nix
, src
}:

rustPlatform.buildRustPackage {
  pname = "xzar-client";
  version = "0.1.0";
  inherit src;

  cargoLock.lockFile = "${src}/Cargo.lock";
  cargoBuildFlags = [ "-p" "xzar-client" ];

  nativeBuildInputs = [
    pkg-config
    makeWrapper
  ];

  buildInputs = [
    xz
  ];

  # Client needs nix-store and xz/pixz at runtime
  postInstall = ''
    wrapProgram $out/bin/xzar \
      --prefix PATH : ${lib.makeBinPath [ nix xz pixz ]}
  '';

  meta = with lib; {
    description = "CLI client for xzar Nix binary cache";
    license = licenses.mpl20;
  };
}
