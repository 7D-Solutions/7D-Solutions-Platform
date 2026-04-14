# Secret Rotation

This runbook covers the secrets that currently have no dedicated operator guide:

- `JWT_PRIVATE_KEY_PEM` / `JWT_PUBLIC_KEY` / `JWT_PUBLIC_KEY_PREV`
- `SERVICE_AUTH_SECRET`
- per-module `DATABASE_URL` secrets

For the full JWT overlap procedure, see [docs/runbooks/key_rotation.md](../runbooks/key_rotation.md).

## 1. JWT Key Rotation

Use this when modules verify tokens from `JwtVerifier::from_env_with_overlap()` or
`JwtVerifier::from_jwks_url()`.

### Rotation Options

- `JWKS` flow: identity-auth publishes a new signing key, module services refresh from `/.well-known/jwks.json`.
- `Env` flow: update `JWT_PUBLIC_KEY` and keep `JWT_PUBLIC_KEY_PREV` set until the overlap window ends.

### Procedure

1. Add the new signing key in identity-auth.
2. Leave the retiring key available for verification.
3. Roll module services so they pick up the new verifier configuration.
4. Wait at least one token TTL, then remove the previous verification key.

### Verification

- New tokens verify on the new key.
- Old tokens verify during the overlap window.
- Old tokens fail after the previous key is removed.
- The coverage test lives in [platform/security/tests/jwt_verification.rs](../../platform/security/tests/jwt_verification.rs).

## 2. `SERVICE_AUTH_SECRET` Rotation

The current service-auth helper reads a single shared secret. There is no
`*_PREV` overlap path, so this is a coordinated cutover, not a dual-secret
rotation.

### Procedure

1. Generate a new secret value.
2. Update every service that signs or verifies internal service tokens.
3. Roll the deployments together.
4. Verify internal service calls still succeed after the cutover.

### Notes

- Do not leave old and new values mixed across services.
- If a service is still running with the old secret, it will reject tokens from
  services already using the new one.

## 3. Database Password Rotation

Each module has its own database secret, usually behind `DATABASE_URL` or a
module-specific `*_DATABASE_URL`.

### Procedure

1. Create the new database password in the database or secret manager.
2. Update the deployment secret for the affected module.
3. Restart or roll that module so it reconnects with the new password.
4. Verify the module reaches `ready` and can query its database.

### Notes

- Rotate one module at a time when possible.
- Keep the old password available until the new deployment has passed health
  checks and the old connection pool has drained.
