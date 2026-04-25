# 2FA / TOTP Design for 7D Platform

**Bead:** bd-eg7b6  
**Author:** SunnyMountain  
**Date:** 2026-04-25  
**Status:** DRAFT — awaiting James's decisions on open questions (§7)

---

## 1. Current State

Auth lives in `modules/customer-portal/src/http/auth.rs`. The login sequence is:

```
POST /portal/auth/login
  → verify email + password
  → immediately issue access_token + refresh_token
  → return 200
```

`portal_users` has `failed_login_count` and `lock_until` (account lockout) but no MFA fields. Token issuance is atomic with password verification — there is no interstitial challenge state.

**Gap:** A user with a compromised password has no second barrier. The platform has no hook point for MFA because there is no challenge-response round trip.

---

## 2. Login Flow Redesign

### 2.1 The Challenge Round-Trip

Replace the single-step login with a two-step protocol:

**Step 1 — Password verification (existing endpoint, modified)**

```
POST /portal/auth/login
Body: { tenant_id, email, password, device_fingerprint? }

Happy path (MFA not enrolled or tenant has mfa_required=false and user not enrolled):
  → 200  { access_token, refresh_token, token_type: "Bearer" }   ← unchanged for un-enrolled users

Happy path (MFA enrolled, or tenant has mfa_required=true):
  → 202  { challenge_id: UUID, methods: ["totp"], expires_at: ISO8601 }
  ← no tokens issued yet

Trusted device present (HttpOnly cookie `td_fp` matches a live trusted_device row):
  → 200  { access_token, refresh_token, token_type: "Bearer" }   ← MFA skipped for this device
```

**Step 2 — MFA verification (new endpoint)**

```
POST /portal/auth/login/verify
Body: { challenge_id: UUID, method: "totp", code: "123456", remember_device: bool? }

Happy path:
  → 200  { access_token, refresh_token, token_type: "Bearer" }
          Set-Cookie: td_fp=<fingerprint>; HttpOnly; Secure; SameSite=Lax; Max-Age=2592000
              (only if remember_device=true AND tenant allows trusted devices)

Failure (wrong TOTP):
  → 401  { error: "invalid_totp" }
  [increment mfa_challenge failed_attempts; lock challenge after N failures — see §7.3]

Expired challenge:
  → 401  { error: "challenge_expired" }

Recovery code path:
POST /portal/auth/login/verify
Body: { challenge_id: UUID, method: "recovery_code", code: "<plaintext>" }
  → validates against portal_recovery_codes, marks used
  → 200  { access_token, refresh_token, token_type: "Bearer", recovery_codes_remaining: N }
```

### 2.2 Challenge Expiry

MFA challenges expire after **10 minutes** (not configurable — a challenge is a session, not a business object). The `mfa_challenges` table is the source of truth; the server does not rely on client-held state. Expired rows can be purged by a background job.

### 2.3 Decision: Where Does This Code Live?

The bead calls this "platform/identity-auth" but no such module exists today — auth is in `customer-portal`. Two options:

**Option A — Extend customer-portal** (lower friction): add MFA tables and endpoints to `modules/customer-portal`. The portal user model already has the right identity anchor (`portal_users`). Customer-portal is the only consumer today.

**Option B — Extract to a new `identity-auth` module** (higher friction, more correct): create a standalone module that owns all user identity concerns. Other verticals (e.g., a future internal-staff portal) import it. Requires splitting the existing customer-portal auth into a shared layer.

**Recommendation: Option A for v1**, with a clear internal module boundary (keep MFA logic in a `src/mfa/` subdirectory) so extraction to its own crate is mechanical when a second vertical needs it. This avoids a large refactor blocking the feature.

---

## 3. Schema Additions

All tables belong in the `customer-portal` database for v1 (see §2.3 decision). They should be added as separate timestamped migrations.

### 3.1 `portal_mfa_enrollments`

```sql
CREATE TABLE portal_mfa_enrollments (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL,
    user_id         UUID NOT NULL REFERENCES portal_users(id) ON DELETE CASCADE,
    method          TEXT NOT NULL CHECK (method IN ('totp')),
    -- TOTP-specific (NULL for future WebAuthn rows)
    secret_enc      TEXT,          -- AES-256 encrypted TOTP secret (key from platform secrets store)
    -- Shared
    label           TEXT,          -- user-visible label ("My iPhone", "Work Laptop")
    enrolled_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at    TIMESTAMPTZ,
    UNIQUE (user_id, method)       -- one enrollment per method per user (v1)
);
CREATE INDEX idx_portal_mfa_enrollments_user ON portal_mfa_enrollments(user_id);
```

`secret_enc` uses the same AES-256 envelope used for OAuth tokens (see `pgp_sym_encrypt` pattern in `modules/integrations/`). The plaintext secret is never stored.

### 3.2 `portal_recovery_codes`

```sql
CREATE TABLE portal_recovery_codes (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   UUID NOT NULL,
    user_id     UUID NOT NULL REFERENCES portal_users(id) ON DELETE CASCADE,
    code_hash   TEXT NOT NULL,     -- bcrypt or SHA-256 HMAC of the plaintext code
    used_at     TIMESTAMPTZ,       -- NULL = available, non-NULL = consumed
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_portal_recovery_codes_user ON portal_recovery_codes(user_id)
    WHERE used_at IS NULL;
```

10 codes generated at enrollment. Each is a random 8-character alphanumeric string (e.g., `A3K7-P9QM`) formatted for readability. Codes are single-use — `used_at` is set atomically on consumption. After all 10 are consumed, the user must re-enroll via admin reset.

### 3.3 `portal_mfa_challenges`

```sql
CREATE TABLE portal_mfa_challenges (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL,
    user_id         UUID NOT NULL REFERENCES portal_users(id) ON DELETE CASCADE,
    method          TEXT NOT NULL CHECK (method IN ('totp', 'recovery_code')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at      TIMESTAMPTZ NOT NULL,       -- created_at + 10 minutes
    consumed_at     TIMESTAMPTZ,                -- NULL = pending
    failed_attempts INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_portal_mfa_challenges_pending ON portal_mfa_challenges(id)
    WHERE consumed_at IS NULL;
```

A challenge is consumed exactly once. The verify endpoint sets `consumed_at` atomically before issuing tokens; retrying with the same `challenge_id` after consumption returns 401.

### 3.4 `portal_trusted_devices`

```sql
CREATE TABLE portal_trusted_devices (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           UUID NOT NULL,
    user_id             UUID NOT NULL REFERENCES portal_users(id) ON DELETE CASCADE,
    device_fingerprint  TEXT NOT NULL,  -- HMAC-SHA256 of cookie value; cookie holds raw token
    issued_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at          TIMESTAMPTZ NOT NULL,
    last_seen_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at          TIMESTAMPTZ
);
CREATE INDEX idx_portal_trusted_devices_user ON portal_trusted_devices(user_id)
    WHERE revoked_at IS NULL;
CREATE UNIQUE INDEX idx_portal_trusted_devices_fp ON portal_trusted_devices(device_fingerprint)
    WHERE revoked_at IS NULL;
```

The cookie holds a random 32-byte token. The server stores `HMAC-SHA256(token)` — never the raw token. On login, the cookie value is hashed server-side and looked up. Sliding expiry: `last_seen_at` and `expires_at` are extended by `remember_me_days` on each successful use.

### 3.5 `portal_tenant_mfa_settings`

```sql
CREATE TABLE portal_tenant_mfa_settings (
    tenant_id               UUID PRIMARY KEY REFERENCES portal_tenants(id) ON DELETE CASCADE,
    mfa_required            BOOLEAN NOT NULL DEFAULT FALSE,
    factor_allowlist        TEXT[] NOT NULL DEFAULT ARRAY['totp'],
    session_lifetime_hours  INTEGER NOT NULL DEFAULT 8 CHECK (session_lifetime_hours BETWEEN 1 AND 168),
    remember_me_days        INTEGER NOT NULL DEFAULT 30 CHECK (remember_me_days BETWEEN 0 AND 365),
    remember_me_enabled     BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

Defaults match James's stated preferences. `remember_me_days=0` disables trusted devices for that tenant even if the feature is globally on.

> **Note:** There is no `portal_tenants` table in the current schema — tenants are referenced by UUID in existing tables. If no such table exists, the FK can be dropped and `tenant_id` stands alone. This should be verified before the migration bead is written.

---

## 4. Self-Enroll Flow

### 4.1 Trigger

Self-enrollment is required for first-admin bootstrap (temp-password login) and optional for existing users via the portal's security settings. Both use the same endpoints.

The login handler detects: user authenticated with a temp password AND no MFA enrollment exists AND tenant `mfa_required=true` → return `202 { require_enrollment: true }` instead of tokens. The client redirects to the enrollment screen.

### 4.2 Endpoints

```
POST /portal/auth/mfa/enroll/totp
  Auth: short-lived enrollment JWT (issued by login step, valid 15 min, scope: enroll_only)
  Body: { label?: string }

Response 200:
  {
    secret: "<base32 plaintext — display once>",
    otpauth_url: "otpauth://totp/7D%20Platform:user%40example.com?secret=...&issuer=7D+Platform",
    qr_code_svg: "<inline SVG>"    -- generated server-side, no external image service
  }
```

The plaintext secret is held in memory only; it is encrypted and written to `portal_mfa_enrollments` only after the user confirms a valid TOTP code (step 2). If the user abandons enrollment, no row is written.

```
POST /portal/auth/mfa/enroll/totp/verify
  Auth: enrollment JWT
  Body: { code: "123456" }

Response 200 (enrollment confirmed):
  {
    recovery_codes: ["A3K7-P9QM", "B8L2-R4TN", ...],  -- 10 codes, shown once
    enrolled_at: "2026-04-25T..."
  }
```

On success:
1. Encrypt the TOTP secret → insert `portal_mfa_enrollments` row.
2. Hash 10 random codes → insert 10 `portal_recovery_codes` rows.
3. Both writes are in a single transaction; the plaintext codes are returned in the response and never stored.
4. Invalidate the enrollment JWT.

If the TOTP code is invalid, return 401 and do not commit. The user can retry.

### 4.3 Re-enrollment / Admin Reset

An admin can call `DELETE /portal/admin/users/:id/mfa` to wipe all MFA rows for a user (enrollments + recovery codes + challenges + trusted devices). The user must re-enroll on next login. This path is not self-service — admin only.

---

## 5. Trusted-Device Lifecycle

### 5.1 Issuance

On successful MFA verify, if `remember_device=true` in the request AND `tenant.remember_me_enabled=true`:

1. Generate `raw_token = rand_bytes(32)`.
2. Set `fingerprint = HMAC-SHA256(TRUSTED_DEVICE_HMAC_KEY, raw_token)`.
3. Insert `portal_trusted_devices` row (`fingerprint`, `expires_at = now() + remember_me_days`).
4. Set response cookie: `Set-Cookie: td_fp=<base64url(raw_token)>; HttpOnly; Secure; SameSite=Lax; Max-Age=<seconds>`.

### 5.2 Verification on Login

When `POST /portal/auth/login` receives a request with a `td_fp` cookie:

1. Hash the cookie value → `candidate_fp`.
2. Look up `portal_trusted_devices` where `device_fingerprint = candidate_fp` AND `user_id = authenticated_user.id` AND `revoked_at IS NULL` AND `expires_at > now()`.
3. Match found → skip MFA challenge, issue tokens directly, slide the expiry.
4. No match (expired, revoked, wrong user) → proceed to MFA challenge as normal. Clear the stale cookie.

### 5.3 Revocation

- **User-initiated:** Session management UI lists trusted devices by `label` (or user-agent at issuance). `DELETE /portal/auth/trusted-devices/:id` sets `revoked_at`.
- **Logout:** Logout revokes the refresh token but does NOT revoke trusted devices by default (the device itself wasn't compromised, just the session). Revocation is explicit.
- **Admin reset:** `DELETE /portal/admin/users/:id/mfa` revokes all trusted devices for the user.
- **Password change:** All trusted devices are revoked when the user changes their password (assume device may be compromised if credentials changed).

---

## 6. Per-Tenant Configurability

The control-plane exposes admin API endpoints to read and write `portal_tenant_mfa_settings`:

```
GET  /portal/admin/tenant/mfa-settings
PUT  /portal/admin/tenant/mfa-settings
Body: {
  mfa_required: bool,
  factor_allowlist: ["totp"],
  session_lifetime_hours: 1–168,
  remember_me_enabled: bool,
  remember_me_days: 0–365
}
```

These endpoints require an admin-scoped token. Vertical admin UIs can drive this API. Changes take effect on the next login (existing sessions are not immediately terminated — that's a scope-creep concern for v2).

---

## 7. Open Questions for James

These four questions need answers before any implementation bead can be created. None of them have safe defaults that wouldn't surprise a security reviewer.

### 7.1 WebAuthn (passkey/hardware key) — v1 or v2?

The schema includes `method TEXT CHECK (method IN ('totp'))` today. Adding WebAuthn in v2 is a schema migration (add `'webauthn'` to the check constraint, add `public_key` and `credential_id` columns). The enrollment and verify endpoints are different enough that a separate code path is required.

**Tradeoff:** WebAuthn is phishing-resistant (TOTP is not). Enterprise security teams prefer WebAuthn. But implementing it in v1 roughly doubles the MFA surface area. The Rust ecosystem has `webauthn-rs` which is mature but complex.

**Recommendation:** Defer to v2 unless Fireproof's customer explicitly requires it.

### 7.2 SMS Fallback — Yes or No?

SMS OTP is widely expected in enterprise software but is the weakest second factor (SIM-swap attacks, SS7 vulnerabilities). It also requires a Twilio/SNS integration.

**Options:**
- Skip SMS entirely (TOTP + recovery codes only)
- Allow SMS as a fallback factor behind a tenant feature flag
- Block SMS at the platform level and let verticals wire it externally

**Recommendation:** Skip for v1. If Fireproof's customer asks for it, add it under a tenant flag in v2.

### 7.3 Lockout After N Failed MFA Attempts

Currently `portal_users` has `failed_login_count` / `lock_until` for password failures. MFA failures are a separate counter (on `portal_mfa_challenges.failed_attempts`).

**Decisions needed:**
- How many consecutive failed TOTP codes before locking the challenge? (Recommended: 5)
- Does locking the challenge lock the account, or just expire the challenge? (Recommendation: expire the challenge, force a fresh login — harder to lock out legitimate users with fat-finger TOTP)
- Is there an account-level MFA failure counter (across challenges)? (Recommendation: no for v1 — let challenge-level expiry do the work)

### 7.4 Per-Role MFA Requirement

Two options:
- **Flat tenant setting:** `mfa_required` applies to all users in the tenant equally.
- **Role-based:** `mfa_required=true` for admin-scoped users, optional for read-only users.

Role-based is more nuanced and requires the platform to know about portal roles (which currently exist only implicitly via JWT scopes). Flat tenant setting is simpler and covers the most common case.

**Recommendation:** Flat tenant setting in v1. Role-based in v2 if a customer requires it.

---

## 8. Recovery Codes

- **Count:** 10 codes per enrollment.
- **Format:** `XXXXXX-XXXXXX` (12 random alphanumeric chars split for readability).
- **Storage:** HMAC-SHA256 of the plaintext code, keyed with `RECOVERY_CODE_HMAC_KEY` from the platform secrets store. Not bcrypt — codes are long enough that timing attack resistance comes from the HMAC, and bcrypt is unnecessarily slow for this use case.
- **Display:** Returned exactly once in the `POST .../enroll/totp/verify` response. The UI must instruct the user to download or print them. After the HTTP response completes, the plaintext is gone.
- **Consumption:** Atomic: `UPDATE portal_recovery_codes SET used_at = now() WHERE id = $1 AND used_at IS NULL RETURNING id`. If no row returned, the code is invalid or already used.
- **Exhaustion:** When `SELECT COUNT(*) FROM portal_recovery_codes WHERE user_id = $1 AND used_at IS NULL` returns 0, the user cannot recover without admin intervention. The verify endpoint returns a `recovery_codes_remaining` count in the success response so the client can warn the user.
- **Regeneration:** Not self-service. Admin resets MFA → user re-enrolls → new codes generated.

---

## 9. Implementation Beads (Priority Order)

These beads should be created after James answers the open questions in §7. Each bead is a single concern.

| # | Bead Title | Priority | Depends On | Notes |
|---|-----------|----------|-----------|-------|
| 1 | `feat(portal): portal_tenant_mfa_settings migration` | P1 | — | Schema only; no logic |
| 2 | `feat(portal): portal_mfa_enrollments + recovery_codes migrations` | P1 | 1 | Schema only |
| 3 | `feat(portal): portal_mfa_challenges migration` | P1 | 1 | Schema only |
| 4 | `feat(portal): portal_trusted_devices migration` | P1 | 1 | Schema only |
| 5 | `feat(portal): TOTP enroll endpoints` | P1 | 2 | `POST .../enroll/totp`, `.../enroll/totp/verify`; uses `totp-rs` crate |
| 6 | `feat(portal): MFA challenge state in login handler` | P1 | 3, 5 | Modify existing login to return 202 when MFA required |
| 7 | `feat(portal): MFA verify endpoint` | P1 | 3, 5, 6 | `POST .../login/verify`; TOTP + recovery code paths |
| 8 | `feat(portal): trusted-device cookie issuance + verification` | P2 | 4, 7 | Cookie set on verify success; skip-challenge on login |
| 9 | `feat(portal): trusted-device revocation API` | P2 | 8 | `DELETE .../trusted-devices/:id` |
| 10 | `feat(portal): admin MFA reset endpoint` | P2 | 5, 8 | `DELETE /portal/admin/users/:id/mfa` |
| 11 | `feat(portal): tenant mfa-settings admin API` | P2 | 1 | `GET/PUT .../admin/tenant/mfa-settings` |
| 12 | `feat(portal): mfa_challenges cleanup job` | P3 | 3 | Purge expired consumed challenges; run as a periodic task |

### Rust Crates

- `totp-rs` — TOTP generation and verification (RFC 6238 compliant, actively maintained)
- `qrcode` — QR code generation as SVG (no external service)
- `base32` — TOTP secret encoding
- `hmac` + `sha2` — recovery code and trusted-device fingerprint hashing

---

## 10. Migration Strategy for Existing Users

### 10.1 Grace Period (Opt-In Initially)

`portal_tenant_mfa_settings.mfa_required` defaults to `false`. Existing users can log in without MFA until a tenant admin enables enforcement. This means:

- Existing users: zero disruption at deploy time.
- New users: invited into a tenant where `mfa_required=false` → no MFA prompt until enforcement changes.
- Enforcement ramp: the tenant admin flips `mfa_required=true` → on next login, users without an enrollment get a `require_enrollment: true` response and are routed through §4.

### 10.2 First-Login Enrollment UX

When a user first hits the `require_enrollment: true` response:
1. The client shows an enrollment screen with a QR code (drives `POST .../enroll/totp`).
2. User scans, enters a code, confirms (drives `POST .../enroll/totp/verify`).
3. Recovery codes are shown and user must acknowledge.
4. Enrollment completes → tokens issued → user proceeds.

This is a one-time friction event. There is no grace period where users can skip enrollment after enforcement is turned on — the invariant (MFA-enrolled users must complete MFA) must hold from the moment the tenant flips the flag.

### 10.3 Admin Bootstrap

First-admin scenario: tenant provisioned, admin invited via temp password email, no MFA yet. On first login:
- Password verified → temp password detected → enrollment required regardless of `mfa_required` flag.
- Enrollment flow as above.
- After enrollment, admin sets up the tenant (including `mfa_required` policy for other users).

---

## 11. Security Considerations

- **TOTP window:** Accept codes from the current 30-second window ± 1 window (90-second tolerance) to handle clock skew. Do not widen further.
- **Secret storage:** The TOTP secret must never appear in logs, error messages, or audit events. Log enrollment events without the secret field.
- **Replay prevention:** A consumed TOTP code (for a given user) must not be reusable within the same TOTP window. Track the last used TOTP timestamp per enrollment in `portal_mfa_enrollments.last_used_at`; reject codes from the same window as `last_used_at`.
- **Challenge isolation:** A `challenge_id` is scoped to a `user_id`. A user cannot use another user's challenge_id — the verify endpoint validates both fields.
- **Trusted device cookie:** `HttpOnly` (no JS access), `Secure` (HTTPS only), `SameSite=Lax` (CSRF-safe for top-level nav). The cookie value is a random token, not a JWT — no algorithm confusion attacks.
- **Recovery code timing:** The lookup uses `constant_time_eq` when comparing HMAC digests to prevent timing oracle attacks.
- **Secret encryption key rotation:** The AES-256 key for `secret_enc` should be stored in the platform secrets store (same pattern as OAuth tokens). Key rotation requires re-encrypting all enrollment secrets — plan for a rotation migration if needed.
