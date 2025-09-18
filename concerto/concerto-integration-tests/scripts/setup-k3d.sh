#!/usr/bin/env bash
# Setup k3d cluster for Concerto integration testing

set -euo pipefail

CLUSTER_NAME="concerto-test"
KUBECONFIG_PATH="$HOME/.kube/k3d-${CLUSTER_NAME}"

echo "Setting up k3d cluster for Concerto integration tests..."

# Check if k3d is installed
if ! command -v k3d &> /dev/null; then
    echo "Error: k3d is not installed. Please install k3d first."
    echo "Visit: https://k3d.io/#installation"
    exit 1
fi

# Check if cluster already exists
if k3d cluster list | grep -q "^${CLUSTER_NAME}"; then
    echo "Cluster ${CLUSTER_NAME} already exists."
    read -p "Do you want to delete and recreate it? (y/n) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        echo "Deleting existing cluster..."
        k3d cluster delete "${CLUSTER_NAME}"
    else
        echo "Using existing cluster."
        export KUBECONFIG="${KUBECONFIG_PATH}"
        kubectl cluster-info
        exit 0
    fi
fi

# Create k3d cluster
echo "Creating k3d cluster '${CLUSTER_NAME}'..."
k3d cluster create "${CLUSTER_NAME}" \
    --servers 1 \
    --agents 2 \
    --port "30080:80@loadbalancer" \
    --port "30443:443@loadbalancer" \
    --port "15432:30432@loadbalancer" \
    --k3s-arg "--disable=traefik@server:0" \
    --kubeconfig-update-default=false \
    --kubeconfig-switch-context=false \
    --wait

# Export kubeconfig
echo "Exporting kubeconfig to ${KUBECONFIG_PATH}..."
k3d kubeconfig get "${CLUSTER_NAME}" > "${KUBECONFIG_PATH}"
export KUBECONFIG="${KUBECONFIG_PATH}"

# Verify cluster is ready
echo "Verifying cluster..."
kubectl cluster-info

# Create namespaces
echo "Creating test namespaces..."
kubectl create namespace integration-tests --dry-run=client -o yaml | kubectl apply -f -

# Apply infrastructure manifests
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MANIFESTS_DIR="${SCRIPT_DIR}/../k8s-manifests"

echo "Deploying PostgreSQL..."
kubectl apply -f "${MANIFESTS_DIR}/postgres.yaml"

echo "Deploying Nostr relay..."
kubectl apply -f "${MANIFESTS_DIR}/nostr-relay.yaml"

# Wait for deployments
echo "Waiting for services to be ready..."
kubectl wait --for=condition=ready pod -l app=postgres --timeout=60s || true
kubectl wait --for=condition=ready pod -l app=nostr-relay --timeout=60s || true

# Create NodePort service for PostgreSQL access from host
cat <<EOF | kubectl apply -f -
apiVersion: v1
kind: Service
metadata:
  name: postgres-nodeport
  namespace: default
spec:
  type: NodePort
  ports:
  - port: 5432
    targetPort: 5432
    nodePort: 30432
  selector:
    app: postgres
EOF

echo ""
echo "✅ k3d cluster '${CLUSTER_NAME}' is ready!"
echo ""
echo "Cluster information:"
echo "  KUBECONFIG: ${KUBECONFIG_PATH}"
echo "  PostgreSQL: localhost:15432 (user: postgres, password: postgres)"
echo "  Nostr Relay: ws://localhost:30080/ws (via ingress)"
echo ""
echo "To use this cluster:"
echo "  export KUBECONFIG=${KUBECONFIG_PATH}"
echo ""
echo "To run integration tests:"
echo "  cargo run -p concerto-integration-tests -- run"
echo ""
echo "To delete the cluster:"
echo "  k3d cluster delete ${CLUSTER_NAME}"