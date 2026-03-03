# ADR-017: External Portal Identity Boundary

**Date:** 2026-03-03
**Status:** Accepted
**Deciders:** Platform Team
**Technical Story:** bd-30lc7 — Customer portal external identity boundary decision

## Context and Problem Statement

The 7D Solutions Platform needs a customer portal where external users (customers,
suppliers, partners) can view documents, acknowledge receipts, check order/invoice
status, and interact with tenant data in a limited, read-mostly capacity.

The platform already has an internal identity service (`identity-auth`) that
manages employee/operator credentials, RBAC, SoD policies, session leases, and
JWT-based access tokens with audience `7d-platform`. This system is designed for
trusted internal actors who operate within the full platform UI.

External portal users are fundamentally different:

- They are **not employees** of the tenant. They represent outside organisations.
- They must **never** receive internal roles/permissions or be treated as internal actors.
- Their access is scoped to a specific **customer/party relationship**, not the full tenant.
- The attack surface is the public internet, not a VPN or internal network.
- Credential lifecycle (invite, reset, disable) is driven by the tenant, not by the user.

Reusing the internal identity system for portal users would collapse the trust
boundary between internal operators and external parties, complicating separation
of duties, audit, and privilege escalation prevention.

## Decision Drivers

* External users must not be able to escalate to internal privileges
* Tenant isolation must be absolute — no cross-tenant data leakage
* Access must be scoped to a specific party (customer/supplier), not the full tenant
* Audit trail must clearly distinguish internal vs external actions
* Portal is internet-facing — higher rate-limiting and abuse protection required
* Must support future federation (OIDC/SAML) without redesigning the core model
* Must not add operational burden of a separate identity provider deployment

## Considered Options

* **Option A:** Reuse identity-auth with a "portal" role
* **Option B:** Separate portal-local credential store with its own JWT issuer
* **Option C:** External IdP only (Auth0, Keycloak) with no local credentials

## Decision Outcome

Chosen option: **Option B — Separate portal-local credential store with its own JWT issuer**, because it maintains a clear trust boundary between internal and external identity while keeping the system self-contained and operationally simple.

### Positive Consequences

* Complete separation of internal and external identity stores — no shared credentials table
* Portal JWT uses a distinct issuer (`portal-auth`) and audience (`7d-portal`) — internal services reject portal tokens by default
* Party-scoped access is a first-class concept, not a bolt-on
* Future OIDC federation is additive (link external IdP identity to portal user) without changing the core model
* Audit events are clearly tagged with `actor_type: "portal_user"` — no ambiguity

### Negative Consequences

* Two credential stores to operate (mitigated: portal store is simpler — no RBAC, no SoD)
* Two JWT key pairs to rotate (mitigated: same rotation pattern, different env vars)
* Portal users cannot "upgrade" to internal users without explicit reprovisioning (this is a feature, not a bug)

## Portal Authentication Model

### Credential Store

The customer portal maintains its own `portal_credentials` table in its own
database (separate from `identity-auth`). Schema:

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID PK | Portal user ID (distinct namespace from internal `user_id`) |
| `tenant_id` | UUID | Owning tenant |
| `party_id` | UUID | Linked party (customer/supplier) from `party` module |
| `email` | VARCHAR(255) | Login email (unique per tenant) |
| `password_hash` | TEXT | Argon2id hash |
| `display_name` | VARCHAR(255) | Human-readable name |
| `is_active` | BOOLEAN | Tenant can disable without deleting |
| `invited_by` | UUID | Internal user who created the invite |
| `invited_at` | TIMESTAMPTZ | When the invite was issued |
| `activated_at` | TIMESTAMPTZ NULL | When the user first set their password |
| `last_login_at` | TIMESTAMPTZ NULL | Last successful login |
| `created_at` | TIMESTAMPTZ | Row creation |
| `updated_at` | TIMESTAMPTZ | Last modification |

**Unique constraints:** `(tenant_id, email)`, `(tenant_id, party_id, email)`

Portal users are created via **invite-only** flow: an internal user (with
appropriate permission) creates the portal user record and triggers an email
with a one-time activation link. Self-registration is not supported.

### Password Policy

- Minimum 12 characters, at least one uppercase, one lowercase, one digit
- Argon2id hashing (same algorithm as internal, independent parameters)
- Password reset via email token (15-minute TTL, single-use)
- Account lockout after 5 failed attempts (30-minute window)

### Rate Limiting

Portal endpoints face the public internet. Rate limits are stricter than internal:

| Endpoint | Limit |
|----------|-------|
| Login | 5 attempts / minute / email |
| Password reset request | 3 / hour / email |
| Token refresh | 10 / minute / token |
| API calls (authenticated) | 60 / minute / user |

### Session / Token Shape

Portal issues its own JWT access tokens with a **distinct issuer and audience**:

```
Issuer:   "portal-auth"
Audience: "7d-portal"
Algorithm: RS256
TTL:      15 minutes (access), 7 days (refresh)
```

Portal refresh tokens follow the same rotation pattern as internal (revoke-on-use,
hash stored, new token issued on refresh).

### Portal Access Token Claims (PortalAccessClaims)

```json
{
  "sub": "<portal_user_id>",
  "iss": "portal-auth",
  "aud": "7d-portal",
  "iat": 1709424000,
  "exp": 1709424900,
  "jti": "<unique-token-id>",
  "tenant_id": "<tenant-uuid>",
  "party_id": "<party-uuid>",
  "actor_type": "portal_user",
  "scopes": ["documents.read", "orders.read", "acknowledgments.write"],
  "ver": "1"
}
```

**Key differences from internal `AccessClaims`:**

| Field | Internal | Portal |
|-------|----------|--------|
| `iss` | `auth-rs` | `portal-auth` |
| `aud` | `7d-platform` | `7d-portal` |
| `actor_type` | `user` / `service` / `system` | `portal_user` |
| `roles` / `perms` | Full RBAC | Not present |
| `party_id` | Not present | Required — scopes access to one party |
| `scopes` | Not present | Granular, portal-specific scope strings |

### Portal Scopes

Portal users do not use the internal RBAC permission system. Instead, they have
**scopes** — a flat list of capabilities assigned per portal user:

| Scope | Description |
|-------|-------------|
| `documents.read` | View documents shared with their party |
| `documents.acknowledge` | Submit acknowledgment/signature on documents |
| `orders.read` | View purchase orders linked to their party |
| `invoices.read` | View invoices linked to their party |
| `shipments.read` | View shipment status for their party |
| `quality.read` | View quality records linked to their party |
| `acknowledgments.write` | Submit acknowledgments (receipts, approvals) |

Scopes are assigned per portal user by the tenant admin. They are **not**
inherited from the party record.

## Identity Mapping Model

### External User → Tenant + Party

Every portal user is linked to exactly one `(tenant_id, party_id)` pair:

```
portal_user.tenant_id  →  tenant_registry.id
portal_user.party_id   →  party.parties.id  (where party.tenant_id = portal_user.tenant_id)
```

This mapping is **immutable after creation**. A portal user cannot be reassigned
to a different party. If a person changes organisations, a new portal user is
created and the old one is deactivated.

### Access Scoping

All portal service queries MUST include both `tenant_id` AND `party_id` in their
WHERE clauses. The portal service never queries data outside the user's party.

```sql
-- Every portal query follows this pattern:
SELECT ... FROM documents
WHERE tenant_id = $1 AND party_id = $2 AND ...
```

### Revocation

Portal access is revoked by:
1. **Deactivating the portal user** (`is_active = false`) — immediate, login fails
2. **Revoking all refresh tokens** for the user — forces re-authentication
3. **Deactivating the party** in the party module — cascading effect on all portal users for that party

## Security Invariants

### SI-1: No Trust Boundary Crossing

Portal tokens are **never accepted** by internal platform services. Internal
services validate `aud: "7d-platform"` — portal tokens carry `aud: "7d-portal"`
and are rejected.

Conversely, internal tokens are never accepted by the portal service.

### SI-2: No Privilege Escalation

Portal users cannot:
- Receive internal roles or permissions
- Access the internal RBAC system
- Be referenced as `actor_type: "user"` in any event
- Call any internal API endpoint

There is no "upgrade path" from portal user to internal user. These are
separate identity stores with no foreign key relationship.

### SI-3: Tenant Isolation

Portal users can only see data belonging to their `tenant_id`. Cross-tenant
queries are impossible because:
- JWT `tenant_id` is verified on every request
- All database queries are parameterised with `tenant_id`
- Row-level security (future) will enforce this at the database layer

### SI-4: Party Isolation

Within a tenant, portal users can only see data linked to their `party_id`.
A portal user for Customer A cannot see documents addressed to Customer B,
even within the same tenant.

### SI-5: Audit Trail Separation

All portal user actions produce events with `actor_type: "portal_user"` and
`actor_id` set to the portal user's UUID (from the portal credential store).
These events are:
- Distinguishable from internal user events at a glance
- Queryable by `actor_type` for compliance reporting
- Stored in the same event infrastructure (NATS JetStream) but tagged clearly

### SI-6: Least Privilege by Default

New portal users start with **no scopes**. The tenant admin must explicitly
grant each scope. The portal service rejects requests for which the user lacks
the required scope.

## Tenant-Boundary Invariants

1. **JWT tenant_id is authoritative.** The portal service extracts `tenant_id`
   from the verified JWT. Request bodies or URL parameters cannot override it.

2. **Party_id is authoritative.** The portal service extracts `party_id` from
   the verified JWT. Portal users cannot query data for a different party.

3. **Database queries are double-scoped.** Every query includes both
   `tenant_id` and `party_id`. Missing either is a bug.

4. **Credential store is tenant-scoped.** Email uniqueness is per-tenant —
   the same email can exist as a portal user in multiple tenants (different
   organisations, same person).

5. **Refresh tokens are tenant-scoped.** A refresh token is only valid for
   the tenant_id it was issued under.

## Threat Model

### Attack Surfaces

| Surface | Threat | Mitigation |
|---------|--------|------------|
| Login endpoint | Credential stuffing | Rate limiting (5/min/email), account lockout (5 failures), Argon2id cost |
| Password reset | Token theft/replay | Single-use token, 15-min TTL, email-only delivery |
| JWT token | Theft/replay | 15-min TTL, RS256 signature, refresh rotation |
| Portal API | Horizontal privilege escalation (access other party's data) | JWT party_id enforced on every query, no user-supplied party_id accepted |
| Portal API | Vertical privilege escalation (gain internal access) | Separate issuer/audience, internal services reject portal tokens |
| Portal API | Cross-tenant access | JWT tenant_id enforced, DB queries double-scoped |
| Invite flow | Invite link interception | One-time activation token, 24-hour TTL, HTTPS only |
| Refresh token | Token reuse after rotation | Revoke-on-use pattern, replay detection with security alert |

### Non-Goals

- **DDoS protection**: Handled at infrastructure layer (CDN/WAF), not application layer
- **Bot detection / CAPTCHA**: Not in scope for initial implementation
- **Multi-factor authentication**: Deferred to a future bead (additive, no redesign needed)
- **Passwordless / magic link login**: Deferred — can be added alongside password auth later
- **External IdP federation (OIDC/SAML)**: Deferred — the identity-linking model supports it (link external sub to portal_user_id) but implementation is a separate bead

## Contract for CP Service Implementation

### Required JWT Claims

The portal service must verify the following claims on every authenticated request:

| Claim | Type | Required | Description |
|-------|------|----------|-------------|
| `sub` | UUID string | Yes | Portal user ID |
| `iss` | string | Yes | Must be `"portal-auth"` |
| `aud` | string | Yes | Must be `"7d-portal"` |
| `exp` | i64 | Yes | Expiration (Unix timestamp) |
| `iat` | i64 | Yes | Issued at (Unix timestamp) |
| `jti` | UUID string | Yes | Unique token ID |
| `tenant_id` | UUID string | Yes | Owning tenant |
| `party_id` | UUID string | Yes | Linked party |
| `actor_type` | string | Yes | Must be `"portal_user"` |
| `scopes` | string[] | Yes | Granted portal scopes |
| `ver` | string | Yes | Claims schema version |

### Endpoints to Support

The portal service (CP0/CP1) must implement:

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/portal/auth/login` | None | Email + password → access + refresh tokens |
| POST | `/portal/auth/refresh` | None | Refresh token → new access + refresh tokens |
| POST | `/portal/auth/logout` | None | Revoke refresh token |
| POST | `/portal/auth/activate` | None | Set password from invite token |
| POST | `/portal/auth/forgot-password` | None | Request password reset email |
| POST | `/portal/auth/reset-password` | None | Set new password from reset token |
| GET | `/portal/ready` | None | Health check |

Admin endpoints (called by internal users with appropriate permission):

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/portal/admin/users` | Internal JWT | Create/invite portal user |
| GET | `/portal/admin/users` | Internal JWT | List portal users for tenant |
| PATCH | `/portal/admin/users/:id` | Internal JWT | Update scopes, deactivate |
| DELETE | `/portal/admin/users/:id` | Internal JWT | Deactivate portal user |

### Failure Behaviours

| Condition | HTTP Status | Body |
|-----------|-------------|------|
| Invalid/expired access token | 401 | `{"error": "unauthorized"}` |
| Valid token, missing required scope | 403 | `{"error": "forbidden", "required_scope": "..."}` |
| Valid token, wrong tenant/party for resource | 404 | `{"error": "not_found"}` (do not leak existence) |
| Account locked | 423 | `{"error": "account_locked"}` |
| Account deactivated | 403 | `{"error": "account_disabled"}` |
| Rate limited | 429 | `{"error": "rate_limited"}` + `Retry-After` header |
| Internal error | 500 | `{"error": "internal_error"}` (no details leaked) |

### Event Types

Portal actions emit events via the standard EventEnvelope with:
- `actor_type`: `"portal_user"`
- `actor_id`: portal user UUID

| Event | Description |
|-------|-------------|
| `portal.user.invited` | Portal user record created by internal admin |
| `portal.user.activated` | Portal user set their password |
| `portal.user.login` | Successful login |
| `portal.user.login_failed` | Failed login attempt |
| `portal.user.logout` | Explicit logout |
| `portal.user.deactivated` | Account deactivated by admin |
| `portal.user.scopes_updated` | Scopes changed by admin |
| `portal.user.password_reset` | Password was reset |
| `portal.token.refreshed` | Access token refreshed |

## Links

* Internal identity service: `platform/identity-auth/`
* Security claims verifier: `platform/security/src/claims.rs`
* Party module: `modules/party/`
* Portal identity contract types: `platform/platform-contracts/src/portal_identity.rs`
* Downstream beads: bd-1nvtn (CP0 scaffold), bd-3fx3b (CP1 document access)
