#!/usr/bin/env bash
# Teardown k3d cluster for Concerto integration testing

set -euo pipefail

CLUSTER_NAME="concerto-test"
KUBECONFIG_PATH="$HOME/.kube/k3d-${CLUSTER_NAME}"

echo "Tearing down k3d cluster for Concerto integration tests..."

# Check if k3d is installed
if ! command -v k3d &> /dev/null; then
    echo "Error: k3d is not installed."
    exit 1
fi

# Check if cluster exists
if ! k3d cluster list | grep -q "^${CLUSTER_NAME}"; then
    echo "Cluster ${CLUSTER_NAME} does not exist."
    exit 0
fi

# Delete cluster
echo "Deleting k3d cluster '${CLUSTER_NAME}'..."
k3d cluster delete "${CLUSTER_NAME}"

# Remove kubeconfig
if [ -f "${KUBECONFIG_PATH}" ]; then
    echo "Removing kubeconfig file..."
    rm -f "${KUBECONFIG_PATH}"
fi

echo ""
echo "✅ k3d cluster '${CLUSTER_NAME}' has been deleted!"