.PHONY: help e2e e2e-infra-up e2e-infra-down e2e-clean build test

help:
	@echo "7D Solutions Platform - Make Commands"
	@echo ""
	@echo "E2E Testing:"
	@echo "  make e2e              - Run full NATS-based E2E integration test"
	@echo "  make e2e-infra-up     - Start infrastructure (NATS + Postgres)"
	@echo "  make e2e-infra-down   - Stop infrastructure"
	@echo "  make e2e-clean        - Stop infrastructure and clean volumes"
	@echo ""
	@echo "Build & Test:"
	@echo "  make build            - Build all services in release mode"
	@echo "  make test             - Run unit tests"

# Start infrastructure for E2E tests
e2e-infra-up:
	@echo "🚀 Starting E2E infrastructure (NATS + Postgres)..."
	@docker compose -f docker-compose.infrastructure.yml up -d
	@echo "⏳ Waiting for infrastructure to be ready..."
	@sleep 5
	@echo "✓ Infrastructure ready"

# Stop infrastructure
e2e-infra-down:
	@echo "🛑 Stopping E2E infrastructure..."
	@docker compose -f docker-compose.infrastructure.yml down
	@echo "✓ Infrastructure stopped"

# Clean infrastructure (remove volumes)
e2e-clean:
	@echo "🧹 Cleaning E2E infrastructure and volumes..."
	@docker compose -f docker-compose.infrastructure.yml down -v
	@echo "✓ Infrastructure and volumes removed"

# Build all services
build:
	@echo "🔧 Building all services..."
	@cargo build --release
	@echo "✓ All services built"

# Run unit tests
test:
	@echo "🧪 Running unit tests..."
	@cargo test --lib
	@echo "✓ Unit tests complete"

# Run E2E integration test
e2e: e2e-infra-up
	@echo ""
	@echo "🧪 Running NATS-based E2E integration test..."
	@echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
	@echo ""
	@cd e2e-tests && cargo test --test real_e2e -- --ignored --test-threads=1 --nocapture || (echo "\n❌ E2E test failed\n" && exit 1)
	@echo ""
	@echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
	@echo "✅ E2E test passed"
	@echo ""
	@echo "💡 Tip: Run 'make e2e-infra-down' to stop infrastructure"
