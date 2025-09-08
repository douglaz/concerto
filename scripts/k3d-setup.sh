#!/usr/bin/env bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

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

# Check if Docker is running
if ! docker info >/dev/null 2>&1; then
    print_error "Docker is not running. Please start Docker first."
    exit 1
fi

# Use local k3d if available, otherwise system k3d
K3D_BIN="${K3D_BIN:-./bin/k3d}"
if [ ! -x "$K3D_BIN" ]; then
    K3D_BIN="k3d"
fi

# Check if k3d is installed
if ! command -v "$K3D_BIN" &> /dev/null; then
    print_error "k3d is not installed. Please install k3d first."
    print_info "Installation instructions: https://k3d.io/#installation"
    exit 1
fi

# Alias k3d to use the right binary
k3d() {
    "$K3D_BIN" "$@"
}

# Change to project root
cd "$(dirname "$0")/.."

print_info "Starting FeLaaS development environment with k3d..."

# Create network if it doesn't exist
if ! docker network inspect felaas-net >/dev/null 2>&1; then
    print_info "Creating Docker network felaas-net..."
    docker network create felaas-net
fi

# Stop any existing k3d cluster
if k3d cluster list | grep -q felaas-test; then
    print_info "Stopping existing k3d cluster..."
    k3d cluster delete felaas-test
fi

# Note: PostgreSQL will run inside k3d, not as external container
# Remove any leftover PostgreSQL container from old setup
if docker ps -a | grep -q felaas-postgres; then
    print_info "Removing old PostgreSQL container..."
    docker-compose -f scripts/docker-compose.postgres.yml down -v 2>/dev/null || true
fi

# Create k3d cluster
print_info "Creating k3d cluster with 5 nodes (1 server + 4 agents)..."
k3d cluster create --config scripts/k3d-config.yaml

# Get kubeconfig
print_info "Setting up kubeconfig..."
mkdir -p ./k8s-config
k3d kubeconfig get felaas-test > ./k8s-config/kubeconfig.yaml

# Export kubeconfig
KUBECONFIG="$(pwd)/k8s-config/kubeconfig.yaml"
export KUBECONFIG

# Use local kubectl if available
KUBECTL_BIN="${KUBECTL_BIN:-kubectl}"
if ! command -v "$KUBECTL_BIN" &> /dev/null; then
    print_warning "kubectl not found in PATH, will try to download it"
    curl -LO "https://dl.k8s.io/release/v1.28.3/bin/linux/amd64/kubectl"
    chmod +x kubectl
    mv kubectl ./bin/
    KUBECTL_BIN="./bin/kubectl"
fi

# Alias kubectl to use the right binary
kubectl() {
    "$KUBECTL_BIN" "$@"
}

# Wait for nodes to be ready
print_info "Waiting for Kubernetes nodes to be ready..."
timeout 120 bash -c 'until kubectl get nodes | grep -c Ready | grep -q 5; do sleep 5; done' || {
    print_error "Timeout waiting for nodes to be ready"
    kubectl get nodes
    exit 1
}

# Create storage classes for persistent volumes
print_info "Creating storage classes..."
kubectl apply -f - <<EOF
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: local-ebs-sc
provisioner: rancher.io/local-path
reclaimPolicy: Delete
volumeBindingMode: WaitForFirstConsumer
---
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: test-az-ebs-sc
provisioner: rancher.io/local-path
reclaimPolicy: Delete
volumeBindingMode: WaitForFirstConsumer
EOF

# Label nodes with availability zone
print_info "Labeling nodes with availability zones..."
kubectl label node k3d-felaas-test-agent-0 topology.kubernetes.io/zone=local --overwrite
kubectl label node k3d-felaas-test-agent-1 topology.kubernetes.io/zone=local --overwrite
kubectl label node k3d-felaas-test-agent-2 topology.kubernetes.io/zone=local --overwrite
kubectl label node k3d-felaas-test-agent-3 topology.kubernetes.io/zone=local --overwrite

# Install NGINX Ingress Controller
print_info "Installing NGINX Ingress Controller..."
kubectl apply -f https://raw.githubusercontent.com/kubernetes/ingress-nginx/controller-v1.8.2/deploy/static/provider/cloud/deploy.yaml

# Wait for ingress controller to be ready
print_info "Waiting for ingress controller to be ready..."
kubectl wait --namespace ingress-nginx \
  --for=condition=ready pod \
  --selector=app.kubernetes.io/component=controller \
  --timeout=120s || print_warning "Ingress controller is taking longer than expected"

print_success "k3d development environment is ready!"
print_info ""
print_info "Environment Information:"
print_info "  Cluster name: felaas-test"
print_info "  Nodes: 1 server + 4 agents"
print_info "  Kubernetes API: https://localhost:6443"
print_info "  PostgreSQL: localhost:5432 (user: postgres, password: postgres)"
print_info ""
print_info "Kubeconfig has been saved to: ./k8s-config/kubeconfig.yaml"
print_info ""
print_info "To use kubectl with this cluster:"
print_info "  export KUBECONFIG=$(pwd)/k8s-config/kubeconfig.yaml"
print_info ""
print_info "To view cluster info:"
print_info "  kubectl cluster-info"
print_info "  kubectl get nodes"
print_info ""
print_info "To stop the environment:"
print_info "  k3d cluster delete felaas-test"
print_info "  docker-compose -f scripts/docker-compose.postgres.yml down"
print_info ""
print_info "To view logs:"
print_info "  kubectl logs -n <namespace> <pod-name>"
print_info "  docker-compose -f scripts/docker-compose.postgres.yml logs"

# Create convenience script
cat > ./k3d-env.sh << 'EOF'
#!/bin/bash
# Source this file to set up your environment
export KUBECONFIG="$(pwd)/k8s-config/kubeconfig.yaml"
export PGHOST=localhost
export PGPORT=5432
export PGUSER=postgres
export PGPASSWORD=postgres
export PGDATABASE=felaas_integration_tests
echo "Environment configured!"
echo "  KUBECONFIG=$KUBECONFIG"
echo "  PostgreSQL: $PGHOST:$PGPORT"
EOF
chmod +x ./k3d-env.sh

print_info ""
print_info "Run this to configure your shell:"
print_info "  source ./k3d-env.sh"
