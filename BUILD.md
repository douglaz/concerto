# Build Instructions for Concerto

## Current Build Status

### ✅ Working Components

- **guardianito-oss**: Fully compiles and runs
- **SQL migrations**: Complete and ready to use
- **Nix environment**: Properly configured with all dependencies

### ⚠️ Known Issues

#### jemalloc Build Issue (felaas-oss)

The `felaas-oss` component has a compilation issue with `tikv-jemalloc-sys` due to a C compiler incompatibility. This is a known issue with newer compilers and the jemalloc version used by fedimint-rocksdb.

**Error:**
```
error: incompatible pointer to integer conversion returning 'char *' from a function with result type 'int'
```

## Build Instructions

### Option 1: Build Guardianito-OSS Only (Recommended for Testing)

```bash
# Using Nix (recommended)
nix develop -c cargo build --package guardianito-oss

# Or without Nix
cargo build --package guardianito-oss
```

### Option 2: Workaround for FeLaaS-OSS

#### Method A: Use an older compiler
```bash
# Install Rust 1.75 (known to work)
rustup install 1.75
rustup default 1.75
cargo build
```

#### Method B: Skip jemalloc (may impact performance)
```toml
# Add to felaas-oss/Cargo.toml
[features]
default = ["no-jemalloc"]
no-jemalloc = []

# Modify fedimint-rocksdb dependency
fedimint-rocksdb = { git = "...", default-features = false }
```

#### Method C: Fix the C code (temporary patch)
```bash
# Edit the build error manually
# In target/debug/build/tikv-jemalloc-sys-*/out/src/malloc_io.c
# Change line 107 from:
#   return strerror_r(err, buf, buflen);
# To:
#   strerror_r(err, buf, buflen); return 0;

# Then rebuild
cargo build --package felaas-oss
```

## Working Development Flow

### 1. Database Setup

```bash
# Create PostgreSQL databases
sudo -u postgres createuser felaas
sudo -u postgres createdb felaas -O felaas
sudo -u postgres createuser guardianito
sudo -u postgres createdb guardianito -O guardianito

# Run migrations
psql -U felaas -d felaas < sql/migrations/001_initial.sql
psql -U felaas -d felaas < sql/migrations/002_subscriptions.sql
psql -U felaas -d felaas < sql/migrations/003_federations.sql
```

### 2. Run Guardianito-OSS (Working)

```bash
# Generate Nostr keys (if needed)
# You can use any Nostr client to generate nsec/npub keys

# Run the bot
nix develop -c cargo run --package guardianito-oss -- daemon \
  --private-key "nsec1..." \
  --relays wss://relay.damus.io,wss://relay.nostr.info
```

### 3. Run FeLaaS-OSS Components (When Fixed)

```bash
# API Server (requires jemalloc fix)
cargo run --package felaas-oss -- api \
  --bind "[::]:3001" \
  --wallet-federation-invite-code "fed1..." \
  --wallet-federation-db-path "/tmp/felaas-wallet"
```

## Testing

### Test Nostr Bot

1. Use any Nostr client (e.g., Damus, Amethyst, nostr.com)
2. Find the bot's npub (printed on startup)
3. Send a DM with JSON commands:

```json
{"GetStatus": {}}
```

### Integration Tests

```bash
# Run k3d cluster
./scripts/k3d-setup.sh

# Run integration tests (when implemented)
cargo test --package felaas-oss-integration-tests
```

## Development Tips

### Using Nix Shell

Always use the Nix development shell for consistent dependencies:

```bash
# Enter shell
nix develop

# Run commands
cargo build
cargo test
cargo run
```

### Quick Iteration

For quick development of Nostr features:
```bash
# Only rebuild guardianito-oss
nix develop -c cargo watch -x "check -p guardianito-oss"
```

## Future Improvements

1. **Fix jemalloc**: Update to a newer version or patch the build
2. **Alternative allocator**: Consider using mimalloc or system allocator
3. **Split dependencies**: Move fedimint-rocksdb to optional feature
4. **Docker build**: Create containerized build environment

## Support

If you encounter build issues:

1. Check you're using the Nix shell: `nix develop`
2. Try building individual packages first
3. Report issues with full error output
4. Consider using the workarounds above

## Status Summary

- **Guardianito-OSS**: ✅ Ready for production use
- **FeLaaS-OSS**: ⚠️ Requires jemalloc workaround
- **Maestro (Integration Tests)**: 🚧 Not yet implemented
- **SQL Migrations**: ✅ Complete and ready
- **Nix Environment**: ✅ Fully configured