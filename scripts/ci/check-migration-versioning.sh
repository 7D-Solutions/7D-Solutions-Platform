#!/usr/bin/env bash
# Migration Versioning Check
# Ensures migration filenames within each module are:
#   1. Strictly sequential (no gaps or duplicates in the version prefix)
#   2. Valid SQL files with a recognized naming convention
#   3. Not empty (each migration must contain at least one SQL statement)
#
# Supports two naming conventions:
#   - Timestamp-based: 20260210000001_description.sql
#   - Sequential numeric: 001_description.sql
#
# Usage: scripts/ci/check-migration-versioning.sh

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

errors=0
checked=0

# Find all migration directories (skip archive/)
migration_dirs=()
while IFS= read -r dir; do
  migration_dirs+=("$dir")
done < <(find modules/ platform/ products/ -type d -name migrations 2>/dev/null | sort)

if [ ${#migration_dirs[@]} -eq 0 ]; then
  echo -e "${YELLOW}No migration directories found${NC}"
  exit 0
fi

for mig_dir in "${migration_dirs[@]}"; do
  sql_files=()
  while IFS= read -r f; do
    sql_files+=("$f")
  done < <(find "$mig_dir" -maxdepth 1 -name '*.sql' -type f | sort)

  if [ ${#sql_files[@]} -eq 0 ]; then
    continue
  fi

  ((checked++)) || true
  module_label="${mig_dir#./}"

  # Extract version prefixes and check for duplicates
  prefixes=()
  for sql_file in "${sql_files[@]}"; do
    basename=$(basename "$sql_file")

    # Validate filename matches expected pattern
    if ! echo "$basename" | grep -qE '^[0-9]+_.*\.sql$'; then
      echo -e "${RED}✗ $module_label: Invalid migration filename: $basename${NC}"
      echo "  Expected: <number>_<description>.sql"
      ((errors++)) || true
      continue
    fi

    # Extract numeric prefix
    prefix=$(echo "$basename" | grep -oE '^[0-9]+')
    prefixes+=("$prefix")

    # Check file is not empty (ignoring comments and whitespace)
    content=$(grep -vE '^\s*--' "$sql_file" | grep -vE '^\s*$' || true)
    if [ -z "$content" ]; then
      echo -e "${RED}✗ $module_label: Empty migration: $basename${NC}"
      ((errors++)) || true
    fi
  done

  # Check for duplicate prefixes
  if [ ${#prefixes[@]} -gt 0 ]; then
    dupes=$(printf '%s\n' "${prefixes[@]}" | sort | uniq -d)
    if [ -n "$dupes" ]; then
      echo -e "${RED}✗ $module_label: Duplicate migration version(s): $dupes${NC}"
      ((errors++)) || true
    fi
  fi

  # Check ordering: prefixes must be strictly increasing
  if [ ${#prefixes[@]} -gt 1 ]; then
    prev=""
    for p in "${prefixes[@]}"; do
      if [ -n "$prev" ]; then
        # Compare as strings (works for both conventions since they're zero-padded)
        if [[ ! "$p" > "$prev" ]]; then
          echo -e "${RED}✗ $module_label: Migration order violation: $prev >= $p${NC}"
          ((errors++)) || true
        fi
      fi
      prev="$p"
    done
  fi

  # For sequential (short) prefixes, check for gaps
  if [ ${#prefixes[@]} -gt 0 ]; then
    first="${prefixes[0]}"
    if [ ${#first} -le 4 ]; then
      # Sequential convention — check for gaps
      expected=$(printf '%0*d' ${#first} 1)
      for p in "${prefixes[@]}"; do
        if [ "$p" != "$expected" ]; then
          echo -e "${RED}✗ $module_label: Gap in sequential migrations: expected $expected, found $p${NC}"
          ((errors++)) || true
          break
        fi
        next=$((10#$p + 1))
        expected=$(printf '%0*d' ${#first} "$next")
      done
    fi
  fi
done

echo ""
echo "Checked $checked migration directories"

if [ $errors -eq 0 ]; then
  echo -e "${GREEN}✓ All migration versions are valid and sequential${NC}"
  exit 0
else
  echo -e "${RED}✗ Found $errors migration versioning error(s)${NC}"
  exit 1
fi
