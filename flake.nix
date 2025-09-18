{
  description = "Federation Tools OSS - Bitcoin-only FeLaaS and Nostr-based Guardianito";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };

        nativeBuildInputs = with pkgs; [
          rustToolchain
          pkg-config
          protobuf
          clang
          llvmPackages.libclang
          cmake
          gnumake
          gcc
        ];

        buildInputs = with pkgs; [
          openssl
          postgresql
          rocksdb
          zstd
          jemalloc
        ] ++ lib.optionals stdenv.isDarwin [
          darwin.apple_sdk.frameworks.Security
          darwin.apple_sdk.frameworks.SystemConfiguration
        ];

        devTools = with pkgs; [
          cargo-watch
          cargo-edit
          cargo-outdated
          cargo-audit
          cargo-nextest
          bacon
          just
          tokio-console
          sqlx-cli
          docker-compose
          kubectl
          k9s
          k3d
        ];

      in
      {
        devShells.default = pkgs.mkShell {
          inherit nativeBuildInputs;
          
          buildInputs = buildInputs ++ devTools;
          
          shellHook = ''
            echo "Federation Tools OSS Development Environment"
            echo "============================================="
            echo ""
            echo "Available commands:"
            echo "  cargo build         - Build the project"
            echo "  cargo test          - Run tests"
            echo "  cargo run           - Run the application"
            echo "  cargo watch         - Watch for changes and rebuild"
            echo "  just                - Run project tasks"
            echo ""
            echo "Services:"
            echo "  felaas-oss          - Bitcoin-only FeLaaS"
            echo "  guardianito-oss     - Nostr-based guardian bot"
            echo ""
            
            # Set up environment variables for development
            export RUST_LOG=info
            export RUST_BACKTRACE=1
            export CARGO_NET_GIT_FETCH_WITH_CLI=true
            
            # PostgreSQL development settings
            export PGHOST=localhost
            export PGPORT=5432
            
            # Ensure rocksdb can be found
            export ROCKSDB_LIB_DIR="${pkgs.rocksdb}/lib"
            export ROCKSDB_INCLUDE_DIR="${pkgs.rocksdb}/include"
            
            # Set LIBCLANG_PATH for bindgen
            export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
          '';

          # Environment variables
          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
        };

        packages = rec {
          felaas-oss = pkgs.rustPlatform.buildRustPackage {
            pname = "felaas-oss";
            version = "0.1.0";
            
            src = ./.;
            
            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "fedimint-api-client-0.7.2" = "sha256-PLACEHOLDER";
              };
            };
            
            inherit nativeBuildInputs buildInputs;
            
            buildAndTestSubdir = "felaas-oss";
            
            meta = with pkgs.lib; {
              description = "Open Source Fedimint as a Service - Bitcoin-only subscriptions";
              homepage = "https://github.com/yourusername/federation-tools-oss";
              license = licenses.mit;
            };
          };

          guardianito-oss = pkgs.rustPlatform.buildRustPackage {
            pname = "guardianito-oss";
            version = "0.1.0";
            
            src = ./.;
            
            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "fedimint-api-client-0.7.2" = "sha256-PLACEHOLDER";
              };
            };
            
            inherit nativeBuildInputs buildInputs;
            
            buildAndTestSubdir = "guardianito-oss";
            
            meta = with pkgs.lib; {
              description = "Open Source Fedimint Guardian Bot - Nostr-based coordination";
              homepage = "https://github.com/yourusername/federation-tools-oss";
              license = licenses.mit;
            };
          };

          default = felaas-oss;
        };

        apps = rec {
          felaas-oss = flake-utils.lib.mkApp {
            drv = self.packages.${system}.felaas-oss;
          };
          
          guardianito-oss = flake-utils.lib.mkApp {
            drv = self.packages.${system}.guardianito-oss;
          };
          
          default = felaas-oss;
        };
      }
    );
}