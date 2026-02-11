#!/usr/bin/env bash
set -euo pipefail

echo "Key rotation sim is primarily a PROCEDURE test."
echo "1) Start system with kid=A, mint token."
echo "2) Rotate to kid=B while keeping A public in JWKS (grace)."
echo "3) Verify old token still validates."
echo "4) Remove A from JWKS after grace -> verify old token fails."

echo ""
echo "Commands (example):"
cat <<'CMDS'
# Generate new key
openssl genrsa -out private_B.pem 2048
openssl rsa -in private_B.pem -pubout -out public_B.pem

# Update env:
# JWT_PRIVATE_KEY_PEM -> private_B.pem
# JWT_PUBLIC_KEY_PEM  -> public_B.pem
# JWT_KID -> auth-key-B

# Keep auth-key-A public in JWKS until 2x ACCESS_TOKEN_TTL
CMDS
