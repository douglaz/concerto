{
  description = "Concerto - Distributed Federation Management System with Nostr Coordination";

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
          # Additional dependencies for Nostr and Fedimint
          secp256k1
          sqlite
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
          # Nostr tools for testing
          websocat
        ];

      in
      {
        devShells.default = pkgs.mkShell {
          inherit nativeBuildInputs;
          
          buildInputs = buildInputs ++ devTools;
          
          shellHook = ''
            echo "Concerto Development Environment"
            echo "================================="
            echo ""
            echo "Available commands:"
            echo "  cargo build         - Build the project"
            echo "  cargo test          - Run tests"
            echo "  cargo run           - Run the application"
            echo "  cargo watch         - Watch for changes and rebuild"
            echo "  cargo clippy        - Run linter"
            echo "  cargo fmt           - Format code"
            echo ""
            echo "Crates:"
            echo "  concerto-common     - Shared data models and types"
            echo "  concerto-guardianito - Guardian tool implementation"
            echo "  concerto-felaas     - FeLaaS provider implementation"
            echo ""
            echo "Quick start:"
            echo "  cargo run -p concerto-guardianito -- --help"
            echo "  cargo run -p concerto-felaas -- --help"
            echo ""
            
            # Set up environment variables for development
            export RUST_LOG=info
            export RUST_BACKTRACE=1
            export CARGO_NET_GIT_FETCH_WITH_CLI=true
            
            # PostgreSQL development settings
            export PGHOST=localhost
            export PGPORT=5432
            export DATABASE_URL="postgresql://postgres:postgres@localhost:5432/concerto"
            
            # Ensure rocksdb can be found
            export ROCKSDB_LIB_DIR="${pkgs.rocksdb}/lib"
            export ROCKSDB_INCLUDE_DIR="${pkgs.rocksdb}/include"
            
            # Set LIBCLANG_PATH for bindgen
            export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
            
            # OpenSSL configuration
            export OPENSSL_DIR="${pkgs.openssl.dev}"
            export OPENSSL_LIB_DIR="${pkgs.openssl.out}/lib"
            export OPENSSL_INCLUDE_DIR="${pkgs.openssl.dev}/include"
            
            # SQLite for Fedimint dependencies
            export SQLITE3_LIB_DIR="${pkgs.sqlite.out}/lib"
            export SQLITE3_INCLUDE_DIR="${pkgs.sqlite.dev}/include"
          '';

          # Environment variables
          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig:${pkgs.sqlite.dev}/lib/pkgconfig";
        };

        packages = rec {
          concerto-guardianito = pkgs.rustPlatform.buildRustPackage {
            pname = "concerto-guardianito";
            version = "0.1.0";
            
            src = ./.;
            
            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "fedimint-api-client-0.7.2" = "sha256-PLACEHOLDER";
                "fedimint-core-0.7.2" = "sha256-PLACEHOLDER";
              };
            };
            
            inherit nativeBuildInputs buildInputs;
            
            buildAndTestSubdir = "concerto-guardianito";
            
            meta = with pkgs.lib; {
              description = "Concerto Guardian Tool - Nostr-based federation management";
              homepage = "https://github.com/your-org/concerto";
              license = licenses.mit;
            };
          };

          concerto-felaas = pkgs.rustPlatform.buildRustPackage {
            pname = "concerto-felaas";
            version = "0.1.0";
            
            src = ./.;
            
            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "fedimint-api-client-0.7.2" = "sha256-PLACEHOLDER";
                "fedimint-core-0.7.2" = "sha256-PLACEHOLDER";
              };
            };
            
            inherit nativeBuildInputs buildInputs;
            
            buildAndTestSubdir = "concerto-felaas";
            
            meta = with pkgs.lib; {
              description = "Concerto FeLaaS - Economically independent federation provider";
              homepage = "https://github.com/your-org/concerto";
              license = licenses.mit;
            };
          };

          default = concerto-guardianito;
        };

        apps = rec {
          concerto-guardianito = flake-utils.lib.mkApp {
            drv = self.packages.${system}.concerto-guardianito;
          };
          
          concerto-felaas = flake-utils.lib.mkApp {
            drv = self.packages.${system}.concerto-felaas;
          };
          
          default = concerto-guardianito;
        };
      }
    );
}