.PHONY: help build test clean docker-build docker-up docker-down k8s-deploy k8s-delete

# Default target
help:
	@echo "Concerto - Distributed Federation Management via Nostr"
	@echo ""
	@echo "Available targets:"
	@echo "  make build          - Build all components"
	@echo "  make test           - Run all tests"
	@echo "  make clean          - Clean build artifacts"
	@echo "  make docker-build   - Build Docker images"
	@echo "  make docker-up      - Start Docker Compose stack"
	@echo "  make docker-down    - Stop Docker Compose stack"
	@echo "  make k8s-deploy     - Deploy to Kubernetes"
	@echo "  make k8s-delete     - Remove from Kubernetes"
	@echo "  make dev            - Run development environment"
	@echo "  make prod           - Deploy production environment"

# Build targets
build:
	@echo "Building Concerto components..."
	cargo build --release

test:
	@echo "Running tests..."
	cargo test --all

clean:
	@echo "Cleaning build artifacts..."
	cargo clean
	rm -rf target/

# Docker targets
docker-build:
	@echo "Building Docker images..."
	docker build -f Dockerfile.guardianito -t concerto/guardianito:latest .
	docker build -f Dockerfile.felaas -t concerto/felaas:latest .

docker-up:
	@echo "Starting Docker Compose stack..."
	docker-compose up -d

docker-down:
	@echo "Stopping Docker Compose stack..."
	docker-compose down

docker-logs:
	docker-compose logs -f

# Kubernetes targets
k8s-deploy:
	@echo "Deploying to Kubernetes..."
	kubectl apply -f kubernetes/namespace.yaml
	kubectl apply -f kubernetes/secrets-template.yaml
	kubectl apply -f kubernetes/postgres-statefulset.yaml
	kubectl apply -f kubernetes/felaas-deployment.yaml
	kubectl apply -f kubernetes/guardianito-statefulset.yaml
	kubectl apply -f kubernetes/monitoring.yaml

k8s-delete:
	@echo "Removing from Kubernetes..."
	kubectl delete -f kubernetes/

k8s-status:
	@echo "Checking Kubernetes status..."
	kubectl get all -n concerto

# Development environment
dev:
	@echo "Starting development environment..."
	@if [ ! -f .env ]; then cp .env.example .env; fi
	docker-compose up postgres -d
	@echo "Waiting for PostgreSQL to start..."
	@sleep 5
	DATABASE_URL=postgres://felaas:changeme@localhost/felaas cargo run --bin felaas -- init-db
	@echo "Starting services..."
	cargo run --bin guardianito -- daemon &
	cargo run --bin felaas -- daemon &
	@echo "Development environment ready!"

# Production deployment
prod: docker-build
	@echo "Deploying production environment..."
	docker-compose -f docker-compose.yml up -d
	@echo "Production deployment complete!"
	@echo "Access Grafana at http://localhost:3000"
	@echo "Access FeLaaS API at http://localhost:8080"

# Database operations
db-init:
	@echo "Initializing database..."
	DATABASE_URL=postgres://felaas:changeme@localhost/felaas cargo run --bin felaas -- init-db

db-migrate:
	@echo "Running database migrations..."
	DATABASE_URL=postgres://felaas:changeme@localhost/felaas sqlx migrate run

# Monitoring
monitor:
	@echo "Opening monitoring dashboards..."
	@open http://localhost:3000 || xdg-open http://localhost:3000

# Generate keys for testing
generate-keys:
	@echo "Generating test Nostr keys..."
	@cargo run --bin guardianito -- generate-keys

# Check code quality
check:
	@echo "Running code quality checks..."
	cargo fmt --check
	cargo clippy -- -D warnings
	cargo test --all

# Format code
fmt:
	@echo "Formatting code..."
	cargo fmt --all

# Update dependencies
update:
	@echo "Updating dependencies..."
	cargo update

# Install development tools
install-tools:
	@echo "Installing development tools..."
	cargo install sqlx-cli
	cargo install cargo-watch
	cargo install cargo-audit

# Security audit
audit:
	@echo "Running security audit..."
	cargo audit

# Coverage report
coverage:
	@echo "Generating coverage report..."
	cargo tarpaulin --out Html --output-dir target/coverage

# Documentation
docs:
	@echo "Building documentation..."
	cargo doc --no-deps --open

# CI/CD pipeline simulation
ci: check test audit
	@echo "CI pipeline complete!"