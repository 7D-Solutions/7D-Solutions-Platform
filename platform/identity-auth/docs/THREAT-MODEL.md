# THREAT-MODEL — auth-rs v1.4.x (7D Solutions Modules)

## Scope
auth-rs provides:
- Credential storage (password_hash)
- Refresh token issuance + rotation + revocation
- Access token issuance (RS256 JWT)
- JWKS publication (/.well-known/jwks.json)
- Security telemetry + best-effort NATS publish

Out of scope:
- User profiles, roles, permissions, tenancy metadata (reference-rs owns)
- Authorization decisions

## Assets
1) JWT private key (catastrophic if compromised)
2) Credential DB (password_hash + refresh token hashes)
3) Refresh tokens in transit (client storage risk)
4) Access tokens in transit
5) Logs/metrics (detection + forensics)

## Trust boundaries
Internet → reverse proxy/TLS → auth-rs → Postgres
auth-rs → NATS JetStream

## Threats & Mitigations

### Brute force / credential stuffing
- Per-tenant/email rate limiting
- Lockout threshold + duration
- Telemetry and alerts on failure/lockout spikes

### Argon2 DoS (RAM/CPU)
- Semaphore concurrency limiter
- Acquire timeout → 503 "auth busy"
- Metrics for hash_busy and alerts for sustained triggers

### Refresh replay
- Rotation + revoke-on-use
- Replay detection logs with client_ip and user_agent
- Metric increment + alert if > 0

### DB compromise
- Argon2 strong parameters
- No plaintext refresh tokens stored
- Separate private key custody policy

### JWT private key compromise
- Separate env secrets (prod/stage different keys)
- Key rotation playbook
- Immediate rotation and forced re-auth on suspected compromise

### User enumeration
- Generic 401 for unknown email vs wrong password
- Avoid timing differences where practical
- Rate limits

### Dependency outage
- DB outage → readiness false; block sensitive operations
- NATS outage → best-effort publish only; never block auth flows

## Explicitly accepted risks
1) Access tokens are not instantly revocable (TTL-based)
2) Rate limiting is per-node in multi-instance deployments unless centralized
3) auth-rs does not validate user/tenant existence (reference-rs responsibility)

## Required controls for production
- TLS termination at proxy
- Separate keys per environment
- Prometheus scraping + alerts loaded
- Regular key rotation simulation
