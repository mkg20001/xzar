{ lib
, rustPlatform
, pkg-config
, postgresql
, dioxus-cli
, wasm-bindgen-cli
, binaryen
, rustToolchain
, llvmPackages
, src
}:

rustPlatform.buildRustPackage {
  pname = "xzar-server";
  version = "0.1.0";
  inherit src;

  cargoLock.lockFile = "${src}/Cargo.lock";
  cargoBuildFlags = [ "-p" "xzar-server" "--features" "embed-ui" ];

  nativeBuildInputs = [
    pkg-config
    dioxus-cli
    wasm-bindgen-cli
    binaryen
    rustToolchain
    llvmPackages.lld
  ];

  buildInputs = [
    postgresql.lib
  ];

  preBuild = ''
    dx build --release --package xzar-ui
  '';

  # Skip tests as they require client binary and postgres
  doCheck = false;

  meta = with lib; {
    description = "A pinning-based Nix cache server";
    license = licenses.mit;
  };
}
