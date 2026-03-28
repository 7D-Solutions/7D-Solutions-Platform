#!/usr/bin/env bash
# Proof script for compliance-export module
# Verifies: build, clippy clean, tests pass
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

echo "=== compliance-export proof ==="
echo ""

echo "[1/3] Building compliance-export..."
./scripts/cargo-slot.sh build -p compliance-export --quiet
echo "    Build: OK"

echo "[2/3] Running clippy..."
./scripts/cargo-slot.sh clippy -p compliance-export --quiet -- -D warnings
echo "    Clippy: OK"

echo "[3/3] Running tests..."
./scripts/cargo-slot.sh test -p compliance-export --quiet
echo "    Tests: OK"

echo ""
echo "=== compliance-export proof PASSED ==="
