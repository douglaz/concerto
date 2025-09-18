#!/usr/bin/env bash
set -euo pipefail

# Integration test runner for FeLaaS
# This script handles the complete lifecycle of integration tests including
# environment setup, test execution, and cleanup.

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
CLUSTER_NAME="felaas-test"
POSTGRES_COMPOSE="scripts/docker-compose.postgres.yml"
KUBECONFIG_PATH="k8s-config/kubeconfig.yaml"

# Use local k3d if available
K3D_BIN="${K3D_BIN:-./bin/k3d}"
if [ ! -x "$K3D_BIN" ]; then
    K3D_BIN="k3d"
fi

# Default options
CLEANUP_AFTER=false
SETUP_ONLY=false
TEST_ONLY=false
CI_MODE=false
TEST_FILTER=""
VERBOSE=false
KEEP_ON_FAILURE=false
TEST_MODULE=""
SHOW_SUMMARY=true

# Function to print colored output
print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

# Function to show usage
usage() {
    cat << EOF
Usage: $0 [OPTIONS]

Run FeLaaS integration tests with automatic environment setup and cleanup.

OPTIONS:
    -h, --help              Show this help message
    -c, --cleanup           Clean up environment after tests
    -s, --setup-only        Only set up environment, don't run tests
    -t, --test-only         Only run tests, assume environment is ready
    -f, --filter PATTERN    Run only tests matching PATTERN
    -m, --module MODULE     Run only tests from specific module (k3d_test, federation_idempotency)
    -v, --verbose           Show detailed output
    -k, --keep-on-failure   Keep environment running if tests fail (for debugging)
    --no-summary            Don't show test execution summary
    --ci                    Run in CI mode (implies --cleanup)

ENVIRONMENT VARIABLES:
    TEST_VERBOSE=1          Enable verbose output
    TEST_TIMEOUT_SECS=300   Set test timeout in seconds
    TEST_PGDATABASE=name    Override test database name
    TEST_PARALLEL=0         Disable parallel test execution

EXAMPLES:
    # Run all integration tests with setup
    $0

    # Run tests and clean up after
    $0 --cleanup

    # Run specific test
    $0 --filter test_federation_deployment

    # Run tests from specific module
    $0 --module k3d_test

    # Debug failed tests (keep environment)
    $0 --keep-on-failure --verbose

    # Only set up environment
    $0 --setup-only

    # Run in CI
    $0 --ci

EOF
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -h|--help)
            usage
            exit 0
            ;;
        -c|--cleanup)
            CLEANUP_AFTER=true
            shift
            ;;
        -s|--setup-only)
            SETUP_ONLY=true
            shift
            ;;
        -t|--test-only)
            TEST_ONLY=true
            shift
            ;;
        -f|--filter)
            TEST_FILTER="$2"
            shift 2
            ;;
        -m|--module)
            TEST_MODULE="$2"
            shift 2
            ;;
        -v|--verbose)
            VERBOSE=true
            export TEST_VERBOSE=1
            shift
            ;;
        -k|--keep-on-failure)
            KEEP_ON_FAILURE=true
            shift
            ;;
        --no-summary)
            SHOW_SUMMARY=false
            shift
            ;;
        --ci)
            CI_MODE=true
            CLEANUP_AFTER=true
            shift
            ;;
        *)
            print_error "Unknown option: $1"
            usage
            exit 1
            ;;
    esac
done

# Change to project root
cd "$(dirname "$0")/.."

# Function to check if command exists
command_exists() {
    command -v "$1" &> /dev/null
}

# Function to run command with proper environment
run_cmd() {
    # Replace k3d with our binary path
    local cmd="$1"
    shift
    if [ "$cmd" = "k3d" ]; then
        "$K3D_BIN" "$@"
    elif [ "$CI_MODE" = true ] || ! command_exists nix; then
        # In CI or without nix, assume we're already in nix develop
        "$cmd" "$@"
    else
        # In local development, use nix develop if needed
        if [ -n "${IN_NIX_SHELL:-}" ]; then
            "$cmd" "$@"
        else
            nix develop -c "$cmd" "$@"
        fi
    fi
}

# Function to check if k3d cluster is running
is_cluster_running() {
    "$K3D_BIN" cluster list 2>/dev/null | grep -q "$CLUSTER_NAME"
}

# Function to check if PostgreSQL is running inside k3d
is_postgres_running() {
    # Check if PostgreSQL StatefulSet exists and is ready
    if [ -f "$KUBECONFIG_PATH" ]; then
        kubectl --kubeconfig="$KUBECONFIG_PATH" get statefulset postgres -n default >/dev/null 2>&1
    else
        return 1
    fi
}

# Function to set up environment
setup_environment() {
    print_info "Setting up integration test environment..."

    # Check Docker
    if ! docker info >/dev/null 2>&1; then
        print_error "Docker is not running. Please start Docker first."
        exit 1
    fi

    # Check if environment is already set up
    local need_k3d=true
    local need_postgres=true

    if is_cluster_running; then
        print_info "k3d cluster '$CLUSTER_NAME' is already running"
        need_k3d=false
    fi

    if is_postgres_running; then
        print_info "PostgreSQL is already running"
        need_postgres=false
    fi

    if [ "$need_k3d" = false ] && [ "$need_postgres" = false ]; then
        print_success "Environment is already set up"

        # Ensure kubeconfig exists
        if [ ! -f "$KUBECONFIG_PATH" ]; then
            print_info "Updating kubeconfig..."
            mkdir -p ./k8s-config
            run_cmd k3d kubeconfig get "$CLUSTER_NAME" > "$KUBECONFIG_PATH"
        fi
        return 0
    fi

    # Run setup script
    print_info "Running k3d setup script..."
    if [ "$VERBOSE" = true ]; then
        run_cmd ./scripts/k3d-setup.sh
    else
        # In CI, show output for debugging
        if [ "$CI_MODE" = true ]; then
            run_cmd ./scripts/k3d-setup.sh
        else
            run_cmd ./scripts/k3d-setup.sh > /dev/null 2>&1
        fi
    fi
    
    # Verify kubeconfig was created
    if [ ! -f "$KUBECONFIG_PATH" ]; then
        print_error "k3d setup did not create kubeconfig at: $KUBECONFIG_PATH"
        print_info "Current directory: $(pwd)"
        print_info "Directory contents:"
        ls -la
        if [ -d k8s-config ]; then
            print_info "k8s-config contents:"
            ls -la k8s-config/
        fi
        exit 1
    fi

    print_success "Environment setup complete"
}

# Function to run tests
run_tests() {
    print_info "Running integration tests..."

    # Export environment variables
    export PGHOST=localhost
    export PGPORT=15432  # NodePort exposed from k3d
    export PGUSER=postgres
    export PGPASSWORD=postgres
    export PGDATABASE=${TEST_PGDATABASE:-felaas_integration_tests}
    # Use absolute path for KUBECONFIG - use pwd since we cd to project root
    KUBECONFIG="$(pwd)/$KUBECONFIG_PATH"
    export KUBECONFIG
    
    # Verify the kubeconfig file exists
    if [ ! -f "$KUBECONFIG" ]; then
        print_error "Kubeconfig file not found at: $KUBECONFIG"
        return 1
    fi
    
    # In CI mode, set environment variable to skip k3d binary check
    if [ "$CI_MODE" = true ]; then
        export CI=true
    fi

    # Track test execution
    local test_start_time=$(date +%s)
    local test_result=0

    # Set PGSCHEMA which doesn't have a default
    export PGSCHEMA=${PGSCHEMA:-public}

    # Run tests - parameters will be picked up from environment variables
    cargo run --bin felaas-integration-tests run
}

# Function to clean up environment
cleanup_environment() {
    print_info "Cleaning up test environment..."

    local had_errors=false

    # Delete k3d cluster
    if is_cluster_running; then
        print_info "Deleting k3d cluster..."
        if ! run_cmd k3d cluster delete "$CLUSTER_NAME" 2>/dev/null; then
            print_warning "Failed to delete k3d cluster"
            had_errors=true
        fi
    fi

    # Note: PostgreSQL runs inside k3d and will be cleaned up with the cluster

    # Remove Docker network
    if docker network inspect felaas-net >/dev/null 2>&1; then
        print_info "Removing Docker network..."
        if ! docker network rm felaas-net 2>/dev/null; then
            print_warning "Failed to remove Docker network"
            had_errors=true
        fi
    fi

    # Remove kubeconfig directory
    if [ -d ./k8s-config ]; then
        rm -rf ./k8s-config
    fi

    # Remove generated env script
    if [ -f ./k3d-env.sh ]; then
        rm -f ./k3d-env.sh
    fi

    if [ "$had_errors" = false ]; then
        print_success "Environment cleaned up successfully"
    else
        print_warning "Environment cleanup completed with some warnings"
    fi
}

# Main execution
main() {
    local exit_code=0

    # Handle setup
    if [ "$TEST_ONLY" = false ]; then
        if ! setup_environment; then
            print_error "Environment setup failed"
            exit 1
        fi
    fi

    # Handle test execution
    if [ "$SETUP_ONLY" = false ]; then
        if ! run_tests; then
            exit_code=1
        fi
    fi

    # Handle cleanup
    if [ "$CLEANUP_AFTER" = true ]; then
        cleanup_environment
    elif [ "$SETUP_ONLY" = false ] && [ "$KEEP_ON_FAILURE" = false ]; then
        print_info ""
        print_info "Test environment is still running. To clean up, run:"
        print_info "  $0 --cleanup"
        print_info ""
        print_info "Or manually:"
        print_info "  k3d cluster delete $CLUSTER_NAME"
        print_info "  docker-compose -f $POSTGRES_COMPOSE down -v"
    fi

    exit $exit_code
}

# Trap to ensure cleanup on script exit (for CI mode)
if [ "$CI_MODE" = true ]; then
    trap cleanup_environment EXIT
fi

# Run main function
main
