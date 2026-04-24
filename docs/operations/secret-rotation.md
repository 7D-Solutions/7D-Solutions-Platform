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

## 3. Avalara AvaTax Credentials Rotation

Avalara issues two credentials per environment (sandbox, production) — an Account ID
and a License Key. Both are treated as equally sensitive; rotate them together,
never separately.

### Where they live

Primary source of truth: Google Secret Manager. Secret names:

- `avalara-sandbox-account-id`
- `avalara-sandbox-license-key`
- `avalara-prod-account-id`
- `avalara-prod-license-key`

Env-var fallback (used when GCP is unreachable or for local dev):
`AVALARA_ACCOUNT_ID` and `AVALARA_LICENSE_KEY`, plus `AVALARA_BASE_URL` to select
sandbox vs prod endpoint.

### Procedure

1. Rotate at Avalara first: log into the Avalara developer portal, reset the
   License Key for the target account (sandbox or prod). Note the old key — you
   need it during the overlap window.
2. Add a new secret version in Google Secret Manager for both `-account-id` and
   `-license-key` (the Account ID rarely changes, but rotate it if Avalara
   reissues one).
3. Roll the modules that read these secrets (today: the module running
   `AvalaraProvider` — typically AR). Startup picks the latest version
   automatically.
4. Verify a live calculation succeeds: run a single invoice through the
   `TaxProvider.quote_tax` path and confirm a non-error response. The test
   command is in `modules/ar/tests/avalara_provider_test.rs` (`#[ignore]` —
   requires credentials).
5. Revoke the old License Key at Avalara only after 24 hours of clean
   operation on the new one. This protects against the brief window where
   in-flight calls still carry the old key in memory.

### Notes

- Avalara rate-limits authentication failures. Rolling the module before the
  new secret version is visible to it will cause a burst of 401s and may
  temporarily lock the account — always bump the Secret Manager version
  first, then roll.
- The sandbox and prod credentials are completely separate; never use sandbox
  creds against the prod endpoint or vice versa. Pair the credential set with
  the matching `AVALARA_BASE_URL` (`sandbox-rest.avatax.com` vs `rest.avatax.com`).

## 4. Database Password Rotation

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
