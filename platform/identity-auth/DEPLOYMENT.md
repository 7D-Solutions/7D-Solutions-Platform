# DEPLOYMENT — auth-rs

## Recommended topology
- Reverse proxy terminates TLS and injects X-Forwarded-For
- auth-rs runs 2+ instances behind load balancer
- PostgreSQL managed with backups
- NATS JetStream for event publish (best-effort)

## Multi-instance notes
- Rate limiting is per-node unless shared store is used
- Hash concurrency limiter is per-node (intended)
- Lockout is DB-backed (global)

## Future: service-to-service
- User JWTs authenticate end-user context
- Internal mTLS recommended to prevent lateral movement
