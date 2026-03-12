#!/usr/bin/env bash
# generate-service-catalog.sh — Auto-generate docs/PLATFORM-SERVICE-CATALOG.md
# from Cargo.toml, docker-compose, and docs directories.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$REPO_ROOT/docs/PLATFORM-SERVICE-CATALOG.md"

# ─── helpers ─────────────────────────────────────────────────────────
extract_field() {
  # Extract a TOML field value from [package] section only (first occurrence)
  local file="$1" field="$2"
  sed -n '/^\[package\]/,/^\[/{/^'"$field"' *= */p;}' "$file" | head -1 | sed 's/^[^"]*"//;s/".*$//'
}

version_proven() {
  local ver="$1"
  local major="${ver%%.*}"
  [ "$major" -ge 1 ] 2>/dev/null && return 0
  return 1
}

# ─── collect service ports from docker-compose.services.yml ──────────
declare -A SVC_PORTS
if [ -f "$REPO_ROOT/docker-compose.services.yml" ]; then
  while IFS='|' read -r svc_name port; do
    SVC_PORTS["$svc_name"]="$port"
  done < <(python3 -c "
import yaml, sys
with open('$REPO_ROOT/docker-compose.services.yml') as f:
    data = yaml.safe_load(f)
for name, svc in data.get('services', {}).items():
    ports = svc.get('ports', [])
    parts = ports[0].split(':') if ports else []
    port = parts[1] if len(parts) >= 2 else (parts[0] if parts else '')
    print(f'{name}|{port}')
")
fi

# ─── collect DB ports from docker-compose.data.yml ───────────────────
declare -A DB_PORTS
if [ -f "$REPO_ROOT/docker-compose.data.yml" ]; then
  while IFS='|' read -r db_name port; do
    DB_PORTS["$db_name"]="$port"
  done < <(python3 -c "
import yaml, sys
with open('$REPO_ROOT/docker-compose.data.yml') as f:
    data = yaml.safe_load(f)
for name, svc in data.get('services', {}).items():
    if name == 'nats':
        continue
    ports = svc.get('ports', [])
    parts = ports[0].split(':') if ports else []
    port = parts[1] if len(parts) >= 2 else (parts[0] if parts else '')
    print(f'{name}|{port}')
")
fi

# ─── map module dir name → service port and db port ──────────────────
get_svc_port() {
  local dir_name="$1"
  echo "${SVC_PORTS[$dir_name]:-}"
}

get_db_port() {
  local dir_name="$1"
  local db_key="${dir_name}-postgres"
  echo "${DB_PORTS[$db_key]:-}"
}

# ─── check for docs ─────────────────────────────────────────────────
has_vision_doc() {
  local dir_name="$1"
  local upper
  upper=$(echo "$dir_name" | tr '[:lower:]' '[:upper:]' | tr '-' '-')
  [ -f "$REPO_ROOT/docs/architecture/${upper}-VISION.md" ] && return 0
  return 1
}

has_revisions() {
  local path="$1"
  [ -f "$REPO_ROOT/$path/REVISIONS.md" ] && return 0
  return 1
}

in_consumer_guide() {
  local dir_name="$1"
  local upper
  upper=$(echo "$dir_name" | tr '[:lower:]' '[:upper:]' | tr '-' '_')
  grep -qi "$dir_name\|$upper" "$REPO_ROOT/docs/consumer-guide/CG-MODULE-APIS.md" 2>/dev/null && return 0
  return 1
}

# ─── build link column ──────────────────────────────────────────────
build_links() {
  local rel_path="$1" dir_name="$2"
  local links=""
  if has_vision_doc "$dir_name"; then
    local upper
    upper=$(echo "$dir_name" | tr '[:lower:]' '[:upper:]')
    links="[Vision](docs/architecture/${upper}-VISION.md)"
  fi
  if has_revisions "$rel_path"; then
    [ -n "$links" ] && links="$links, "
    links="${links}[Revisions](${rel_path}/REVISIONS.md)"
  fi
  if in_consumer_guide "$dir_name"; then
    [ -n "$links" ] && links="$links, "
    links="${links}[CG](docs/consumer-guide/CG-MODULE-APIS.md)"
  fi
  echo "$links"
}

# ─── collect crate info ─────────────────────────────────────────────
PROVEN_SERVICES=()
UNPROVEN_MODULES=()
PLATFORM_LIBS=()
TOOLS=()

for cargo_file in "$REPO_ROOT"/modules/*/Cargo.toml; do
  dir="$(dirname "$cargo_file")"
  rel_path="${dir#$REPO_ROOT/}"
  dir_name="$(basename "$dir")"
  crate_name=$(extract_field "$cargo_file" "name")
  version=$(extract_field "$cargo_file" "version")
  description=$(extract_field "$cargo_file" "description")
  svc_port=$(get_svc_port "$dir_name")
  db_port=$(get_db_port "$dir_name")
  links=$(build_links "$rel_path" "$dir_name")

  row="| $dir_name | $crate_name | $version | $svc_port | $db_port | $description | $links |"

  if version_proven "$version"; then
    PROVEN_SERVICES+=("$row")
  else
    UNPROVEN_MODULES+=("$row")
  fi
done

for cargo_file in "$REPO_ROOT"/platform/*/Cargo.toml; do
  dir="$(dirname "$cargo_file")"
  rel_path="${dir#$REPO_ROOT/}"
  dir_name="$(basename "$dir")"
  crate_name=$(extract_field "$cargo_file" "name")
  version=$(extract_field "$cargo_file" "version")
  description=$(extract_field "$cargo_file" "description")
  svc_port=$(get_svc_port "$dir_name")
  db_port=$(get_db_port "$dir_name")
  links=$(build_links "$rel_path" "$dir_name")

  # Services with ports are deployable; libs are everything else
  if [ -n "$svc_port" ] || [ "$dir_name" = "identity-auth" ]; then
    # identity-auth uses auth-lb port
    if [ "$dir_name" = "identity-auth" ]; then
      svc_port="${SVC_PORTS[auth-lb]:-8080}"
      db_port="${DB_PORTS[auth-postgres]:-5433}"
    fi
    row="| $dir_name | $crate_name | $version | $svc_port | $db_port | $description | $links |"
    if version_proven "$version"; then
      PROVEN_SERVICES+=("$row")
    else
      UNPROVEN_MODULES+=("$row")
    fi
  else
    row="| $dir_name | $crate_name | $version | $description | $links |"
    PLATFORM_LIBS+=("$row")
  fi
done

for cargo_file in "$REPO_ROOT"/tools/*/Cargo.toml; do
  dir="$(dirname "$cargo_file")"
  rel_path="${dir#$REPO_ROOT/}"
  dir_name="$(basename "$dir")"
  crate_name=$(extract_field "$cargo_file" "name")
  version=$(extract_field "$cargo_file" "version")
  description=$(extract_field "$cargo_file" "description")
  links=""
  if has_revisions "$rel_path"; then
    links="[Revisions](${rel_path}/REVISIONS.md)"
  fi

  row="| $dir_name | $crate_name | $version | $description | $links |"
  TOOLS+=("$row")
done

# ─── sort arrays for determinism ────────────────────────────────────
IFS=$'\n' PROVEN_SERVICES=($(printf '%s\n' "${PROVEN_SERVICES[@]}" | sort)); unset IFS
IFS=$'\n' UNPROVEN_MODULES=($(printf '%s\n' "${UNPROVEN_MODULES[@]}" | sort)); unset IFS
IFS=$'\n' PLATFORM_LIBS=($(printf '%s\n' "${PLATFORM_LIBS[@]}" | sort)); unset IFS
IFS=$'\n' TOOLS=($(printf '%s\n' "${TOOLS[@]}" | sort)); unset IFS

# ─── emit ────────────────────────────────────────────────────────────
{
  cat <<'HEADER'
<!-- DO NOT EDIT — generated by scripts/generate-service-catalog.sh -->

# Platform Service Catalog

Complete inventory of all 7D Solutions Platform services, libraries, and tools.

## Infrastructure

| Component | Container | Port | Purpose |
|-----------|-----------|------|---------|
| NATS JetStream | 7d-nats | 4222 (client), 8222 (monitor) | Event bus for all inter-service communication |
| PostgreSQL 16 | per-service | 5433–5454 | Each service has its own isolated database |

HEADER

  echo "## Proven Services (v1.0.0+)"
  echo ""
  echo "Version-bumped on every change. See [docs/VERSIONING.md](docs/VERSIONING.md)."
  echo ""
  echo "| Module | Crate | Version | Port | DB Port | Description | Docs |"
  echo "|--------|-------|---------|------|---------|-------------|------|"
  for row in "${PROVEN_SERVICES[@]}"; do
    echo "$row"
  done
  echo ""

  echo "## Unproven Modules (v0.x.x)"
  echo ""
  echo "No version bump discipline required yet."
  echo ""
  echo "| Module | Crate | Version | Port | DB Port | Description | Docs |"
  echo "|--------|-------|---------|------|---------|-------------|------|"
  for row in "${UNPROVEN_MODULES[@]}"; do
    echo "$row"
  done
  echo ""

  echo "## Platform Libraries"
  echo ""
  echo "Shared crates used by services. Not independently deployed."
  echo ""
  echo "| Library | Crate | Version | Description | Docs |"
  echo "|---------|-------|---------|-------------|------|"
  for row in "${PLATFORM_LIBS[@]}"; do
    echo "$row"
  done
  echo ""

  echo "## Tools"
  echo ""
  echo "CLI tools and test harnesses."
  echo ""
  echo "| Tool | Crate | Version | Description | Docs |"
  echo "|------|-------|---------|-------------|------|"
  for row in "${TOOLS[@]}"; do
    echo "$row"
  done
  echo ""

  cat <<'FOOTER'
## Key Patterns

- **Guard → Mutation → Outbox**: All state changes follow this atomicity pattern. See [CG-MODULE-APIS.md](docs/consumer-guide/CG-MODULE-APIS.md).
- **EventEnvelope**: Constitutional metadata on every event. See [CG-EVENTS.md](docs/consumer-guide/CG-EVENTS.md).
- **Tenant Isolation**: Every query is scoped by `tenant_id` from JWT claims. See [CG-TENANCY.md](docs/consumer-guide/CG-TENANCY.md).
- **RBAC**: Role-based access control enforced at middleware layer. See [CG-AUTH.md](docs/consumer-guide/CG-AUTH.md).
FOOTER
} > "$OUT"

echo "✓ Generated $OUT"
