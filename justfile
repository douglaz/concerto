#!/usr/bin/env just --justfile

# List available recipes
default:
    @just --list

# Build all crates
build:
    cargo build --all

# Build in release mode
build-release:
    cargo build --release --all

# Run tests
test:
    cargo test --all

# Run tests with output
test-verbose:
    cargo test --all -- --nocapture

# Format code
fmt:
    cargo fmt --all

# Check formatting
fmt-check:
    cargo fmt --all -- --check

# Run clippy
clippy:
    cargo clippy --all -- -D warnings

# Run clippy with all targets
clippy-all:
    cargo clippy --all --all-targets --all-features -- -D warnings

# Check everything (format, clippy, test)
check-all: fmt-check clippy test

# Clean build artifacts
clean:
    cargo clean

# Update dependencies
update:
    cargo update

# Run felaas-oss API server
run-felaas-api:
    cargo run -p felaas-oss -- api

# Run felaas-oss federation launcher
run-felaas-launcher:
    cargo run -p felaas-oss -- federation-launcher

# Run felaas-oss subscription daemon
run-felaas-subscription:
    cargo run -p felaas-oss -- subscription-daemon

# Run guardianito-oss daemon
run-guardianito:
    cargo run -p guardianito-oss -- daemon

# Start PostgreSQL containers
start-db:
    docker-compose up -d

# Stop PostgreSQL containers
stop-db:
    docker-compose down

# View PostgreSQL logs
db-logs:
    docker-compose logs -f

# Initialize database schemas
init-db:
    @echo "Initializing databases..."
    @echo "TODO: Add database migration scripts"

# Development setup
dev-setup: start-db
    @echo "Development environment ready"
    @echo "PostgreSQL running on ports 5432 (guardianito) and 5433 (felaas)"

# Full check before committing
final-check: fmt check-all
    @echo "All checks passed!"

# Watch for changes and rebuild
watch:
    cargo watch -x build

# Watch and run tests
watch-test:
    cargo watch -x test

# Build Docker images
docker-build:
    docker build -t felaas-oss:latest -f docker/felaas-oss.Dockerfile .
    docker build -t guardianito-oss:latest -f docker/guardianito-oss.Dockerfile .

# Run with Nix
nix-build:
    nix build

# Enter Nix development shell
nix-shell:
    nix develop

# Run felaas-oss with Nix
nix-run-felaas:
    nix run .#felaas-oss

# Run guardianito-oss with Nix
nix-run-guardianito:
    nix run .#guardianito-oss