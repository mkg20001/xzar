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
    let
      # Overlay that adds xzar packages
      xzarOverlay = final: prev:
        let
          rustBin = (import rust-overlay final prev).rust-bin;
          rustToolchain = rustBin.stable.latest.default.override {
            extensions = [ "rust-src" "rust-analyzer" ];
            targets = [ "wasm32-unknown-unknown" ];
          };
        in {
          xzar-server = final.callPackage ./nix/xzar-server.nix {
            inherit rustToolchain;
            src = self;
          };
          xzar-client = final.callPackage ./nix/xzar-client.nix {
            src = self;
          };
        };
    in
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [
            (import rust-overlay)
            xzarOverlay
          ];
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
          targets = [ "wasm32-unknown-unknown" ];
        };

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
            binaryen
          ];

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
        };

        packages = {
          xzar-server = pkgs.xzar-server;
          xzar-client = pkgs.xzar-client;
          default = pkgs.xzar-server;
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
      # Overlays
      overlays = {
        default = xzarOverlay;
      };

      nixosModules.xzar = import ./module.nix;
    };
}
