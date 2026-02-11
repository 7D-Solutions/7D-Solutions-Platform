#!/usr/bin/env python3
"""
External JWT validation test - simulates another service validating tokens
using only the JWKS endpoint (no shared secrets).
"""

import json
import requests
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.hazmat.backends import default_backend
import jwt
import base64

# Step 1: Fetch JWKS from auth service
print("1. Fetching JWKS from auth-rs...")
jwks_response = requests.get("http://localhost:8081/.well-known/jwks.json")
jwks = jwks_response.json()
print(f"   Found {len(jwks['keys'])} key(s)")

# Step 2: Get a token from auth service
print("\n2. Getting access token from auth-rs...")
login_response = requests.post(
    "http://localhost:8081/api/auth/login",
    json={
        "tenant_id": "123e4567-e89b-12d3-a456-426614174000",
        "email": "testuser@example.com",
        "password": "ValidPassword123"
    }
)
token = login_response.json()["access_token"]
print(f"   Got token: {token[:50]}...")

# Step 3: Extract kid from token header
header_b64 = token.split('.')[0]
padding = 4 - len(header_b64) % 4
if padding != 4:
    header_b64 += '=' * padding
header = json.loads(base64.urlsafe_b64decode(header_b64))
kid = header['kid']
print(f"\n3. Token kid: {kid}")

# Step 4: Find matching key in JWKS
key_data = None
for key in jwks['keys']:
    if key['kid'] == kid:
        key_data = key
        break

if not key_data:
    print("   ❌ FAILED: kid not found in JWKS")
    exit(1)

print(f"   ✅ Found matching key in JWKS")

# Step 5: Convert JWKS to PEM format for validation
def jwk_to_pem(jwk):
    """Convert JWK to PEM public key"""
    from cryptography.hazmat.primitives.asymmetric.rsa import RSAPublicNumbers
    from cryptography.hazmat.primitives import serialization

    # Decode n and e from base64url
    def b64_to_int(b64_str):
        # Add padding
        padding = 4 - len(b64_str) % 4
        if padding != 4:
            b64_str += '=' * padding
        return int.from_bytes(base64.urlsafe_b64decode(b64_str), 'big')

    n = b64_to_int(jwk['n'])
    e = b64_to_int(jwk['e'])

    public_numbers = RSAPublicNumbers(e, n)
    public_key = public_numbers.public_key(default_backend())

    pem = public_key.public_bytes(
        encoding=serialization.Encoding.PEM,
        format=serialization.PublicFormat.SubjectPublicKeyInfo
    )
    return pem

print("\n4. Converting JWKS to PEM for validation...")
public_key_pem = jwk_to_pem(key_data)
print(f"   ✅ Converted JWKS to PEM")

# Step 6: Validate token using ONLY the public key from JWKS
print("\n5. Validating token using ONLY JWKS public key...")
try:
    decoded = jwt.decode(
        token,
        public_key_pem,
        algorithms=['RS256'],
        audience='7d-platform',  # Expected audience
        issuer='auth-rs',        # Expected issuer
        options={'verify_exp': True}
    )
    print(f"   ✅ Token validated successfully!")
    print(f"   Token claims:")
    print(f"      sub: {decoded['sub']}")
    print(f"      tenant_id: {decoded['tenant_id']}")
    print(f"      jti: {decoded['jti']}")

    # Check for missing claims
    missing = []
    if 'iss' not in decoded:
        missing.append('iss (issuer)')
    if 'aud' not in decoded:
        missing.append('aud (audience)')

    if missing:
        print(f"\n   ⚠️  Missing recommended claims: {', '.join(missing)}")

except jwt.ExpiredSignatureError:
    print("   ❌ Token expired")
    exit(1)
except jwt.InvalidTokenError as e:
    print(f"   ❌ Token validation failed: {e}")
    exit(1)

print("\n✅ JWKS FEDERATION TEST PASSED")
print("   Another service can validate tokens using only JWKS endpoint!")
