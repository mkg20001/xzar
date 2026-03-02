{
  description = "xzar - A Nix binary cache server and client";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
          targets = [ "wasm32-unknown-unknown" ];
        };

        commonBuildInputs = with pkgs; [
          pkg-config
        ];

        # Common Rust package build settings
        buildRustPackage = { pname, cargoBuildFlags ? [], buildInputs ? [], ... }@args:
          pkgs.rustPlatform.buildRustPackage (args // {
            inherit pname;
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = commonBuildInputs;
            inherit buildInputs cargoBuildFlags;
          });

        # Test script with all dependencies
        testScript = pkgs.writeShellApplication {
          name = "xzar-integration-tests";
          runtimeInputs = with pkgs; [
            rustToolchain
            pkg-config
            postgresql
            coreutils
            # For client tests
            nix
            xz
            pixz
          ];
          text = builtins.readFile ./scripts/run-integration-tests.sh;
        };
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            # Rust toolchain
            rustToolchain
            cargo-watch
            cargo-edit

            pkg-config
            postgresql
            diesel-cli
            # For client compression
            xz
            pixz

            # Dioxus CLI and WASM tools
            dioxus-cli
            wasm-bindgen-cli
          ];

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
        };

        packages = {
          # Server package
          xzar-server = buildRustPackage {
            pname = "xzar-server";
            cargoBuildFlags = [ "-p" "xzar-server" ];
            buildInputs = with pkgs; [
              postgresql.lib
            ];
            meta = with pkgs.lib; {
              description = "A pinning-based Nix cache server";
              license = licenses.mit;
            };
          };

          # Client package
          xzar-client = buildRustPackage {
            pname = "xzar-client";
            cargoBuildFlags = [ "-p" "xzar-client" ];
            buildInputs = with pkgs; [
              xz
            ];
            # Client needs nix-store and xz/pixz at runtime
            postInstall = ''
              wrapProgram $out/bin/xzar \
                --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.nix pkgs.xz pkgs.pixz ]}
            '';
            nativeBuildInputs = commonBuildInputs ++ [ pkgs.makeWrapper ];
            meta = with pkgs.lib; {
              description = "CLI client for xzar Nix binary cache";
              license = licenses.mpl20;
            };
          };

          # Default is server
          default = self.packages.${system}.xzar-server;

          # Test script package
          integration-tests = testScript;
        };

        # Apps for running with `nix run`
        apps = {
          integration-tests = {
            type = "app";
            program = "${testScript}/bin/xzar-integration-tests";
          };
        };
      }
    ) // {
      nixosModules.xzar = import ./module.nix;
    };
}
