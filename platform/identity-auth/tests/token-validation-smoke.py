#!/usr/bin/env python3
import sys, json, time, requests, jwt
from jwt import PyJWKClient

AUTH_BASE = sys.argv[1] if len(sys.argv) > 1 else "http://localhost:8080"
JWKS = f"{AUTH_BASE}/.well-known/jwks.json"

def main():
    print("JWKS:", JWKS)
    jwk_client = PyJWKClient(JWKS)
    token = input("Paste access token: ").strip()
    signing_key = jwk_client.get_signing_key_from_jwt(token).key
    claims = jwt.decode(token, signing_key, algorithms=["RS256"], audience="7d-platform", options={"require": ["exp","iat","sub"]})
    print(json.dumps(claims, indent=2))

if __name__ == "__main__":
    main()
