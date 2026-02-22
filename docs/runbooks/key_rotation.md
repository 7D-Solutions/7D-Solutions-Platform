# Key Rotation Runbook

**Phase 48 — Production Hardening (last updated: bd-26ro)**

## Purpose

Zero-downtime rotation procedures for secrets that must never be simultaneously
absent. The platform supports an **overlap window**: the new and old secrets are
both active at once so in-flight requests and long-lived tokens are never
rejected mid-rotation.

## Secrets Inventory

| Secret | Location | Overlap support | Env var(s) |
|--------|----------|-----------------|------------|
| JWT signing key (RSA private) | identity-auth | Yes — `JWT_PREV_PUBLIC_KEY_PEM` + `JWT_PREV_KID` | `JWT_PRIVATE_KEY_PEM`, `JWT_PUBLIC_KEY_PEM`, `JWT_KID` |
| JWT verification key (RSA public) | all module services | Yes — `JWT_PUBLIC_KEY_PREV` | `JWT_PUBLIC_KEY` |
| Tilled webhook HMAC secret | payments | Yes — `TILLED_WEBHOOK_SECRET_PREV` | `TILLED_WEBHOOK_SECRET` |
| Seed admin password | tenant-registry | No (bootstrap only; rejected after first use) | `SEED_ADMIN_PASSWORD` |
| Database passwords | per-module | No (rotate via DB + docker-compose restart) | `DATABASE_URL` |

---

## 1. JWT Key Rotation (Zero-Downtime)

Access tokens are RS256-signed by identity-auth and verified by every module
service. Tokens have a default 15-minute TTL. A rotation overlap of **one TTL
cycle (15 min minimum, 30 min recommended)** ensures no valid token is rejected.

### Step 1 — Generate new RSA key pair

```bash
# Generate new 4096-bit RSA private key
openssl genrsa -out jwt_new_private.pem 4096

# Extract public key
openssl rsa -in jwt_new_private.pem -pubout -out jwt_new_public.pem

# Choose a new key ID (use a timestamp for traceability)
NEW_KID="auth-key-$(date +%Y%m%d)"

echo "New key ID: $NEW_KID"
```

### Step 2 — Add new key to identity-auth (overlap start)

Set these env vars on the identity-auth service and perform a **rolling
restart** (or env-var update in your orchestrator):

```bash
# New key (will sign all NEW tokens)
JWT_PRIVATE_KEY_PEM="$(awk '{printf "%s\\n", $0}' jwt_new_private.pem)"
JWT_PUBLIC_KEY_PEM="$(awk '{printf "%s\\n", $0}' jwt_new_public.pem)"
JWT_KID="$NEW_KID"

# Previous key (kept for verification of outstanding old tokens)
JWT_PREV_PUBLIC_KEY_PEM="$(awk '{printf "%s\\n", $0}' jwt_old_public.pem)"
JWT_PREV_KID="auth-key-old"   # whatever the previous KID was
```

Verify the JWKS endpoint now serves both keys:

```bash
curl -s http://localhost:8080/.well-known/jwks.json | jq '.keys | length'
# Expected: 2
curl -s http://localhost:8080/.well-known/jwks.json | jq '[.keys[].kid]'
# Expected: ["auth-key-<new>", "auth-key-<old>"]
```

### Step 3 — Update module services (overlap continues)

All module services read their verification key from `JWT_PUBLIC_KEY` and,
during rotation, `JWT_PUBLIC_KEY_PREV`. Update each service:

```bash
# New primary verification key
JWT_PUBLIC_KEY="$(cat jwt_new_public.pem)"

# Previous key — tokens issued before Step 2 are still valid
JWT_PUBLIC_KEY_PREV="$(cat jwt_old_public.pem)"
```

Perform a **rolling restart** of all module services. They use
`JwtVerifier::from_env_with_overlap()` which reads both vars automatically.

### Step 4 — Wait for overlap window

Wait at least one access token TTL (default 15 min). Outstanding tokens signed
by the old key will expire during this window.

```bash
# Monitor for auth errors during the window
docker logs 7d-auth -f | grep -E "ERROR|InvalidToken|expired"

# Check per-module auth rejection rates in Grafana
# Dashboard: Platform Overview → Auth Failures by Service
```

### Step 5 — Remove previous key (overlap end)

Once the overlap window has passed, clear the old key from all services:

**identity-auth:**
```bash
# Unset the prev key vars
unset JWT_PREV_PUBLIC_KEY_PEM
unset JWT_PREV_KID
# Restart identity-auth
```

**All module services:**
```bash
# Unset the prev verification key
unset JWT_PUBLIC_KEY_PREV
# Rolling restart
```

Verify JWKS now serves only one key:
```bash
curl -s http://localhost:8080/.well-known/jwks.json | jq '.keys | length'
# Expected: 1
```

### Step 6 — Verify

```bash
# Log in and get a fresh token
TOKEN=$(curl -s -X POST http://localhost:8080/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"admin@example.com","password":"...","tenant_id":"..."}' \
  | jq -r '.access_token')

# Verify it works against a module
curl -s -H "Authorization: Bearer $TOKEN" \
  http://localhost:8086/api/ar/invoices | jq '.total // .error'
```

### Rollback

If any service rejects the new key:

1. Restore the old `JWT_PRIVATE_KEY_PEM`, `JWT_PUBLIC_KEY_PEM`, `JWT_KID` on identity-auth.
2. Restore the old `JWT_PUBLIC_KEY` on all module services.
3. Keep `JWT_PUBLIC_KEY_PREV` as the *new* key until tokens issued with it expire.
4. Rolling restart.

---

## 2. Tilled Webhook Secret Rotation (Zero-Downtime)

Tilled sends webhooks signed with HMAC-SHA256. During rotation, both the old
and new secrets must be accepted simultaneously because Tilled may deliver
webhooks for in-flight events signed with the old secret.

### Step 1 — Generate new secret

Generate via Tilled dashboard or API:
```bash
# Tilled generates the new secret — copy it from the dashboard.
# Example (do NOT generate locally — use Tilled's key generation):
NEW_WEBHOOK_SECRET="whsec_<new_value_from_tilled>"
```

### Step 2 — Update Tilled to sign with new secret

In the Tilled dashboard, add the new webhook endpoint secret. **Do not remove
the old secret yet** — Tilled may continue delivering events signed with the
old secret for a window after the change.

### Step 3 — Add both secrets to payments service (overlap start)

Update the payments service env vars and restart:

```bash
TILLED_WEBHOOK_SECRET="$NEW_WEBHOOK_SECRET"
TILLED_WEBHOOK_SECRET_PREV="$OLD_WEBHOOK_SECRET"
```

The payments service tries each secret in order — a webhook signed by either
the new or old secret will be accepted.

Verify the service started with both secrets:
```bash
docker logs 7d-payments | grep -i "webhook\|tilled" | tail -10
```

### Step 4 — Wait for Tilled to stop sending old-signed webhooks

Tilled's transition window is typically 5–15 minutes after updating the secret.
Monitor for failed signature verifications:

```bash
# Check for signature failures in payments logs
docker logs 7d-payments -f 2>&1 | grep "SignatureError\|signature invalid"
```

### Step 5 — Remove old secret (overlap end)

Once no signature failures appear for 30 minutes:

```bash
# Clear the previous secret
unset TILLED_WEBHOOK_SECRET_PREV
# Restart payments service
docker compose -f docker-compose.modules.yml restart payments
```

### Step 6 — Verify

Send a test webhook from the Tilled dashboard and confirm it is accepted:

```bash
# Check payments logs for a successful webhook
docker logs 7d-payments 2>&1 | grep "webhook" | tail -5
```

### Rollback

If signature failures spike after removing the old secret:

1. Restore `TILLED_WEBHOOK_SECRET_PREV` to the old secret.
2. Restart payments.
3. Contact Tilled support to confirm which secret is currently active.

---

## 3. Rehearsal Checklist

Use this checklist before performing a live rotation in production.

### Pre-rotation checks

```bash
# All services healthy
for svc_port in "ar:8086" "payments:8088" "gl:8090"; do
  curl -sf "http://localhost:${svc_port##*:}/api/health" && echo " OK ${svc_port%%:*}"
done

# JWKS is reachable
curl -sf http://localhost:8080/.well-known/jwks.json | jq '.keys[0].kid'

# Auth works end-to-end (replace with valid credentials)
# curl -s -X POST http://localhost:8080/api/auth/login ...
```

### Post-rotation verification (JWT)

```bash
# 1. JWKS serves new key only
curl -s http://localhost:8080/.well-known/jwks.json | jq '.keys | length'

# 2. Fresh token works
# TOKEN=$(curl -s ... | jq -r '.access_token')
# curl -H "Authorization: Bearer $TOKEN" http://localhost:8086/api/health

# 3. No auth errors in module logs for 5 minutes
for svc in ar payments gl; do
  echo "=== $svc ===" && docker logs "7d-${svc}" 2>&1 | grep -c "InvalidToken\|Unauthorized" || true
done
```

### Post-rotation verification (Webhook)

```bash
# Trigger a test event from Tilled dashboard and confirm it lands
docker logs 7d-payments 2>&1 | grep "payment_intent\|webhook" | tail -10

# Check metrics: webhook_verified_total should increment
curl -s http://localhost:8088/metrics | grep webhook_verified
```

---

## 4. Emergency Rotation (Compromise Suspected)

If a key or secret is believed to be compromised, skip the overlap window:

**JWT key compromise:**
1. Immediately generate new keys (Step 1 above).
2. Deploy new `JWT_PRIVATE_KEY_PEM` + `JWT_PUBLIC_KEY_PEM` to identity-auth — **no prev key**.
3. All outstanding tokens (signed by compromised key) become invalid immediately.
4. Users will be logged out and must re-authenticate.
5. Notify users of forced re-login.

**Webhook secret compromise:**
1. Rotate in Tilled dashboard immediately.
2. Update `TILLED_WEBHOOK_SECRET` in payments — **no prev secret**.
3. Any attacker-forged webhooks will be rejected.
4. Legitimate in-flight webhooks may be rejected briefly — monitor and replay from Tilled dashboard if needed.

---

## References

- `platform/identity-auth/src/auth/jwt.rs` — `JwtKeys::with_prev_key()`
- `platform/security/src/claims.rs` — `JwtVerifier::from_env_with_overlap()`
- `modules/payments/src/webhook_signature.rs` — `validate_webhook_signature(secrets: &[&str])`
- `platform/identity-auth/src/config.rs` — `JWT_PREV_PUBLIC_KEY_PEM`, `JWT_PREV_KID`
- `modules/payments/src/config.rs` — `TILLED_WEBHOOK_SECRET_PREV`
- `docs/runbooks/incident_response.md` — webhook failure decision tree
- `docs/runbooks/disaster_recovery.md` — full DR procedure

## Changelog

- **2026-02-22**: Phase 48 — initial key rotation runbook with JWT + Tilled webhook overlap procedures (bd-26ro)
