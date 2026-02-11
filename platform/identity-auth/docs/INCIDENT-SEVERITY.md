# INCIDENT-SEVERITY — auth-rs

## Severity Levels

### SEV-1 (Critical)
Criteria:
- /health/ready failing across all instances
- Authentication unavailable (login/refresh failing broadly)
- JWT signing key compromised (confirmed or strongly suspected)

Actions (first 15 min):
- Page on-call immediately
- Freeze deploys
- Triage: DB? CPU/RAM? TLS? NATS? Secrets?
- If key compromise: rotate keys + invalidate old in JWKS + force re-auth

### SEV-2 (High)
Criteria:
- Refresh replay > 0 (any) OR spike in replay
- Lockout spike > baseline
- 5xx rate > 1% for > 5 min
- Hash busy events sustained

Actions:
- Investigate IPs/agents, block at proxy/WAF
- Confirm rate limits/lockout working
- Consider scaling horizontally or raising MAX_CONCURRENT_HASHES (if legitimate traffic)

### SEV-3 (Medium)
Criteria:
- Elevated 401 rates, normal availability
- NATS publish failures sustained (but auth still works)
- Increased latency p95 > threshold

Actions:
- Check dependencies
- Adjust pool sizes, infra capacity
- Open incident ticket, monitor

### SEV-4 (Low)
Criteria:
- Single instance unhealthy but overall ok
- Minor warning logs, no user impact

Actions:
- Fix during business hours
- Track in backlog

## Communications
- SEV-1/2: Post updates every 30 minutes
- SEV-3: Post updates every 2 hours
- SEV-4: No broadcast required
