# OPS-CHECKLIST — auth-rs

## Pre-deploy
- [ ] TLS at proxy configured
- [ ] DATABASE_URL points to correct DB
- [ ] NATS_URL reachable
- [ ] JWT keys injected via secrets (not env file in prod)
- [ ] JWT_KID unique
- [ ] Prometheus scraping /metrics
- [ ] Alerts loaded
- [ ] JWKS reachable externally (for consumers)
- [ ] Run key rotation sim quarterly

## Routine
Daily:
- [ ] Check replay metric = 0
- [ ] Check lockout spike
- [ ] Check p95 login latency

Weekly:
- [ ] Review 401 baseline
- [ ] Review rate limiting events
- [ ] Verify backups of DB

Quarterly:
- [ ] Key rotation simulation
- [ ] Chaos test: NATS outage, DB outage
