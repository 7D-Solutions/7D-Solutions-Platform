#!/usr/bin/env bash
# doctor.sh — Verify the local developer environment has the required tools.
#
# Usage:
#   ./scripts/dev/doctor.sh
#
# Exit codes:
#   0  All required prerequisites are present.
#   1  One or more prerequisites are missing.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

TARGET="aarch64-unknown-linux-musl"
MUSL_INSTALL_HINT="brew install filosottile/musl-cross/musl-cross"
missing=()

add_missing() {
  missing+=("missing: $1")
}

require_command() {
  local command_name="$1"
  local install_hint="$2"

  if ! command -v "$command_name" >/dev/null 2>&1; then
    add_missing "$install_hint"
  fi
}

require_file_executable() {
  local path="$1"
  local install_hint="$2"

  if [ ! -x "$path" ]; then
    add_missing "$install_hint"
  fi
}

require_docker() {
  if ! command -v docker >/dev/null 2>&1; then
    add_missing "install Docker Desktop"
    return
  fi

  if ! docker info >/dev/null 2>&1; then
    add_missing "start Docker Desktop"
  fi

  if ! docker compose version >/dev/null 2>&1; then
    add_missing "install Docker Compose v2"
  fi
}

require_rust_target() {
  if ! command -v rustup >/dev/null 2>&1; then
    add_missing "install rustup"
    return
  fi

  if ! rustup target list --installed 2>/dev/null | grep -qx "$TARGET"; then
    add_missing "rustup target add $TARGET"
  fi
}

require_musl_cross() {
  if ! command -v aarch64-linux-musl-gcc >/dev/null 2>&1; then
    add_missing "$MUSL_INSTALL_HINT"
  fi
}

require_command python3 "install python3"
require_command curl "install curl"
require_command br "install br (beads_rust)"
require_command cargo-watch "cargo install cargo-watch"
require_docker
require_rust_target
require_musl_cross
require_file_executable "$PROJECT_ROOT/scripts/cargo-slot.sh" "restore scripts/cargo-slot.sh"

if [ "${#missing[@]}" -gt 0 ]; then
  printf '%s\n' "${missing[@]}" >&2
  exit 1
fi

printf '\033[32mready\033[0m\n'
