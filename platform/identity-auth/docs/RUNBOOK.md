# Auth-RS v1.4.0 Runbook

Production operations guide for the 7D Solutions authentication service.

Service: platform/identity-auth
Language: Rust (Axum)
Database: PostgreSQL
Event Bus: NATS JetStream
JWT: RS256 (asymmetric signing)

---

# 1. Quick Start

## Docker Compose

```bash
docker compose up -d
```

Service defaults:

* HTTP: http://localhost:8080
* Postgres: localhost:5433
* NATS: localhost:4222

## Standalone (Local Dev)

```bash
cargo run
```

Required environment variables:

* DATABASE_URL
* NATS_URL
* JWT_PRIVATE_KEY_PEM
* JWT_PUBLIC_KEY_PEM
* JWT_KID

---

# 2. Health Checks

## Liveness

GET `/health/live`

Returns:

```
200 OK
```

Use for container liveness probes.

## Readiness

GET `/health/ready`

Checks:

* PostgreSQL connectivity
* NATS connectivity

Returns:

```
200 OK
```

or

```
503 Service Unavailable
```

Kubernetes readiness probe example:

```yaml
readinessProbe:
  httpGet:
    path: /health/ready
    port: 8080
  initialDelaySeconds: 5
  periodSeconds: 10
```

---

# 3. Endpoints Reference

## POST /api/auth/register

Creates credentials.

Request:

```json
{
  "tenant_id": "uuid",
  "user_id": "uuid",
  "email": "user@example.com",
  "password": "StrongPassword123"
}
```

Success:

```
201 Created
```

Errors:

* 400 Weak password
* 429 Rate limited
* 503 hash_busy

---

## POST /api/auth/login

Request:

```json
{
  "tenant_id": "uuid",
  "email": "user@example.com",
  "password": "StrongPassword123"
}
```

Success:

```json
{
  "access_token": "...",
  "refresh_token": "..."
}
```

Errors:

* 401 invalid credentials
* 423 locked
* 429 rate limited

---

## POST /api/auth/refresh

Request:

```json
{
  "refresh_token": "..."
}
```

Returns new access + refresh tokens.

---

## GET /.well-known/jwks.json

Returns public RSA keys for token verification.

---

## GET /metrics

Prometheus metrics endpoint.

---

# 4. Monitoring & Metrics

Alert on:

* `auth_login_total{result="failure"}`
* `auth_register_total{result="hash_busy"}`
* `auth_refresh_total{result="replay"}`
* `auth_http_request_duration_seconds` p95/p99 > 500ms
* `auth_dependency_up{dep="db"} == 0`
* `auth_dependency_up{dep="nats"} == 0`

---

# 5. Common Failures

| Symptom                 | Cause                      | Fix                            |
| ----------------------- | -------------------------- | ------------------------------ |
| "Pool timed out"        | DB connection exhausted    | Increase max_connections       |
| 503 hash_busy           | Too many concurrent hashes | Increase MAX_CONCURRENT_HASHES |
| Replay detected logs    | Token reuse                | Investigate IP & UA            |
| Address already in use  | Port conflict              | Change PORT env var            |
| NATS connection failure | NATS down                  | Start NATS                     |

---

# 6. Key Rotation (JWT Keys)

1. Generate new keys:

```bash
openssl genrsa -out private.pem 2048
openssl rsa -in private.pem -pubout -out public.pem
```

2. Add new key to JWKS with new kid
3. Update JWT_KID
4. Keep old public key for 2× ACCESS_TOKEN_TTL
5. Remove old key after grace window

---

# 7. Security Events

Watch for:

* `security.refresh_replay_detected`
* `auth.lockout`
* `auth.hash_busy`

Example log:

```json
{
  "level":"WARN",
  "event":"security.refresh_replay_detected",
  "tenant_id":"uuid",
  "user_id":"uuid",
  "client_ip":"1.2.3.4",
  "user_agent":"Mozilla/5.0",
  "trace_id":"uuid"
}
```

---

# 8. Database Migrations

Applied automatically on startup via:

```
sqlx::migrate!()
```

To add migration:

```bash
sqlx migrate add <name>
```

---

# 9. Backup & Recovery

Backup:

* PostgreSQL database
* JWT private key

No need to backup:

* NATS (event bus)

---

# 10. Performance Tuning

Current Argon2:

* Memory: 64MB
* Iterations: 3
* Parallelism: 1

Target:

* 100–300ms per hash

Tune:

* Increase iterations for stronger security
* Monitor memory usage under load
