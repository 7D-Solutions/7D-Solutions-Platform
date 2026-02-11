#!/usr/bin/env bash
set -euo pipefail
BASE="${1:-http://localhost:8080}"

TOKEN="${TOKEN:-}"
if [[ -z "${TOKEN}" ]]; then
  echo "Set TOKEN env var to an access token"
  exit 1
fi

HDR="$(echo "$TOKEN" | cut -d. -f1 | python3 - <<'PY'
import base64,sys,json
s=sys.stdin.read().strip()
pad='='*((4-len(s)%4)%4)
print(base64.urlsafe_b64decode(s+pad).decode())
PY
)"
echo "Header: $HDR" | grep -q '"alg":"RS256"'

PAY="$(echo "$TOKEN" | cut -d. -f2 | python3 - <<'PY'
import base64,sys
s=sys.stdin.read().strip()
pad='='*((4-len(s)%4)%4)
print(base64.urlsafe_b64decode(s+pad).decode())
PY
)"
echo "Claims: $PAY" | grep -q '"tenant_id"'
echo "Claims: $PAY" | grep -q '"sub"'
echo "Claims: $PAY" | grep -q '"iss":"auth-rs@1.4.0"'
echo "Claims: $PAY" | grep -q '"aud":"7d-platform"'

curl -s "$BASE/.well-known/jwks.json" >/dev/null
echo "Contract OK"
