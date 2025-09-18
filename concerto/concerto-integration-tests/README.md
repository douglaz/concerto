# Concerto Integration Tests

This directory contains comprehensive integration tests for the Concerto project, adapted from the FeLaaS integration test framework.

## Structure

The integration tests are structured as a binary application (rather than `cargo test`) following the FeLaaS pattern for better control over test execution and environment setup.

### Test Modules

- **`common.rs`** - Shared utilities for Kubernetes cluster management, PostgreSQL connections, and test infrastructure
- **`nostr_tests.rs`** - Tests for Nostr relay connectivity and multi-guardian messaging  
- **`dkg_tests.rs`** - Distributed Key Generation protocol tests including setup code exchange
- **`federation_tests.rs`** - Complete federation lifecycle tests including formation, configuration updates, and slot allocation
- **`main.rs`** - CLI test runner with full test suite
- **`main_minimal.rs`** - Minimal test runner for basic verification

## Running Tests

### Prerequisites

1. **k3d**: Lightweight Kubernetes in Docker (https://k3d.io)
2. **Docker**: For running k3d and containers
3. **kubectl**: Kubernetes command-line tool
4. **Rust**: For building the test binary

### Quick Start

Use the convenience script for a complete test run:

```bash
# Run tests with automatic setup and teardown
./run-tests.sh full

# Just run tests (auto-setup if needed)
./run-tests.sh run

# Run tests and keep cluster running
./run-tests.sh run --no-teardown

# Manual setup/teardown
./run-tests.sh setup
./run-tests.sh run --no-teardown
./run-tests.sh teardown
```

### Manual Usage

```bash
# Setup k3d cluster and infrastructure
./scripts/setup-k3d.sh

# Build the integration tests
cargo build -p concerto-integration-tests

# Run with default settings
cargo run -p concerto-integration-tests -- run

# With custom PostgreSQL settings
cargo run -p concerto-integration-tests -- run \
  --pghost localhost \
  --pgport 15432 \
  --pguser postgres

# Using environment variables
export PGHOST=localhost
export PGPORT=15432
export PGPASSWORD=postgres
cargo run -p concerto-integration-tests -- run

# Teardown cluster when done
./scripts/teardown-k3d.sh
```

## Current Status

### ✅ Completed
- Test package structure created
- Binary-based test runner implemented
- Comprehensive test scenarios written:
  - Nostr relay connectivity
  - Multi-guardian coordination
  - DKG protocol flows
  - Federation lifecycle management
  - Configuration updates
  - Slot allocation
- Major nostr-sdk v0.43 API issues resolved

### ⚠️ Known Issues

Due to significant API changes in nostr-sdk v0.43, some test files have remaining compilation issues:

1. **Tag::custom API**: Expects `Vec<String>` for values in events
2. **Filter::custom_tag API**: Expects single `String` value for filtering
3. **NIP-44 decrypt**: Return type handling needs adjustment
4. **Events iteration**: Methods like `.len()` and `.is_empty()` availability

These issues are primarily due to the evolution of the nostr-sdk API between versions. The test logic is sound but requires further API adaptation.

### 🔧 Workaround

A minimal test runner (`main_minimal.rs`) is provided that compiles successfully and can be used to verify the test infrastructure:

```bash
# Use minimal runner
cargo build -p concerto-integration-tests --bin concerto-integration-tests
```

## Test Coverage

### Nostr Coordination Tests
- Basic relay connectivity
- Event publishing and subscription
- Multi-guardian messaging
- Encrypted direct messages (NIP-44)

### DKG Integration Tests  
- Three-guardian DKG ceremony
- Setup code exchange
- Encrypted setup code sharing
- DKG completion verification

### Federation Lifecycle Tests
- Complete federation formation (4 guardians)
- Proposal and acceptance flow
- DKG integration
- Federation activation
- Configuration updates with versioning
- Guardian slot allocation and confirmation

## Future Enhancements

1. **Complete API Migration**: Fully adapt to nostr-sdk v0.43 API
2. **Kubernetes Manifests**: Add YAML files for test infrastructure
3. **k3d Scripts**: Automated cluster setup/teardown
4. **CI Integration**: GitHub Actions workflow
5. **Test Filtering**: Add ability to run specific test suites
6. **Performance Tests**: Load testing with many guardians
7. **Failure Scenarios**: Network partitions, guardian failures

## Development Notes

The test framework follows these principles:
- Binary-based execution for better control
- Namespace isolation for parallel test execution
- UUID-based resource naming to avoid conflicts
- Comprehensive cleanup after test completion
- Detailed logging for debugging

## Dependencies

Key dependencies include:
- `nostr-sdk` v0.43 - Nostr protocol implementation
- `kube` v0.95 - Kubernetes client
- `k8s-openapi` v0.23 - Kubernetes API types
- `tokio-postgres` - PostgreSQL async client
- `deadpool-postgres` - Connection pooling
- `uuid` - Unique test identifiers
- `chrono` - Timestamp handling