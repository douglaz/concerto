#!/usr/bin/env bash
# Quick test runner for Concerto integration tests
# This script handles the full lifecycle: setup, run, teardown

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPTS_DIR="${SCRIPT_DIR}/scripts"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Default values
ACTION=""
TEARDOWN_AFTER="yes"
VERBOSE=""

# Help function
show_help() {
    cat << EOF
Usage: $0 [OPTIONS] ACTION

Actions:
    setup       Set up k3d cluster and infrastructure
    run         Run integration tests (sets up if needed)
    teardown    Tear down k3d cluster
    full        Setup, run tests, and teardown

Options:
    --no-teardown    Keep cluster running after tests
    --verbose        Enable verbose output
    -h, --help       Show this help message

Examples:
    $0 run                    # Setup if needed and run tests
    $0 full                   # Complete cycle: setup, run, teardown
    $0 run --no-teardown      # Run tests and keep cluster
    $0 teardown               # Clean up existing cluster

EOF
    exit 0
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        setup|run|teardown|full)
            ACTION="$1"
            shift
            ;;
        --no-teardown)
            TEARDOWN_AFTER="no"
            shift
            ;;
        --verbose)
            VERBOSE="--verbose"
            shift
            ;;
        -h|--help)
            show_help
            ;;
        *)
            echo -e "${RED}Error: Unknown option $1${NC}"
            show_help
            ;;
    esac
done

if [ -z "$ACTION" ]; then
    echo -e "${RED}Error: No action specified${NC}"
    show_help
fi

# Functions
setup_cluster() {
    echo -e "${GREEN}Setting up k3d cluster...${NC}"
    if ! "${SCRIPTS_DIR}/setup-k3d.sh"; then
        echo -e "${RED}Failed to setup k3d cluster${NC}"
        exit 1
    fi
}

check_cluster() {
    if ! k3d cluster list 2>/dev/null | grep -q "concerto-test"; then
        return 1
    fi
    return 0
}

run_tests() {
    echo -e "${GREEN}Building integration tests...${NC}"
    cargo build -p concerto-integration-tests --bin concerto-integration-tests
    
    echo -e "${GREEN}Running integration tests...${NC}"
    
    # Export kubeconfig for the tests
    export KUBECONFIG="$HOME/.kube/k3d-concerto-test"
    
    # Set PostgreSQL connection for tests
    export PGHOST="localhost"
    export PGPORT="15432"
    export PGUSER="postgres"
    export PGPASSWORD="postgres"
    export PGDATABASE="concerto_test"
    
    # Run the tests
    if cargo run -p concerto-integration-tests -- run $VERBOSE; then
        echo -e "${GREEN}✅ Integration tests completed successfully!${NC}"
        return 0
    else
        echo -e "${RED}❌ Integration tests failed${NC}"
        return 1
    fi
}

teardown_cluster() {
    echo -e "${YELLOW}Tearing down k3d cluster...${NC}"
    "${SCRIPTS_DIR}/teardown-k3d.sh"
}

# Main execution
case $ACTION in
    setup)
        setup_cluster
        ;;
    
    run)
        # Check if cluster exists
        if ! check_cluster; then
            echo -e "${YELLOW}Cluster not found, setting up...${NC}"
            setup_cluster
        else
            echo -e "${GREEN}Using existing cluster 'concerto-test'${NC}"
        fi
        
        run_tests
        TEST_RESULT=$?
        
        if [ "$TEARDOWN_AFTER" == "yes" ]; then
            teardown_cluster
        else
            echo -e "${YELLOW}Cluster 'concerto-test' is still running${NC}"
            echo "To teardown manually: $0 teardown"
        fi
        
        exit $TEST_RESULT
        ;;
    
    teardown)
        teardown_cluster
        ;;
    
    full)
        setup_cluster
        run_tests
        TEST_RESULT=$?
        teardown_cluster
        exit $TEST_RESULT
        ;;
esac

echo -e "${GREEN}Done!${NC}"