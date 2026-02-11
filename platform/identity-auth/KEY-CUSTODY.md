# KEY-CUSTODY — JWT signing keys

## Key types
- JWT private key (RS256): HIGH sensitivity
- JWT public key: safe to distribute (JWKS)

## Storage policy (choose one)
Minimum acceptable:
- Private key stored as Docker/K8s secret, mounted read-only
- Never committed to repo
- Separate keys per environment (prod != staging)

Preferred:
- Vault/KMS-managed signing (future)

## Rotation policy
- Rotate at least quarterly or immediately on suspected compromise
- Maintain old public keys in JWKS for 2x ACCESS_TOKEN_TTL
- Remove retired keys after grace window

## Disaster scenarios
1) Private key lost:
- All tokens minted by that key become unverifiable
- Mitigation: store backup in secret manager with access controls
2) Private key compromised:
- Rotate immediately; invalidate old key in JWKS
- Force re-auth; investigate logs, DB access, CI secrets
