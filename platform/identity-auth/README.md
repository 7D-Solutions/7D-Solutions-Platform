# auth-rs v1.4

Authentication service for 7D Solutions modular platform.

## Architecture v1.4 Compliance

- ✅ Independent deployable module
- ✅ PostgreSQL database (no shared tables)
- ✅ Event-driven via NATS JetStream
- ✅ Strict event schema validation
- ✅ RS256 JWT access tokens
- ✅ Refresh token rotation
- ✅ Health checks (/health/live, /health/ready)
- ✅ Distributed tracing (trace_id, causation_id)
- ✅ Fail-fast startup

## What auth-rs Owns

- Credential storage (password hashes)
- Refresh token storage
- Access token signing (RS256)
- Login/logout flows
- Auth domain events

## What auth-rs Does NOT Own

- Tenants (owned by reference-rs)
- User profiles (owned by reference-rs)
- Roles/permissions (owned by reference-rs)

## Setup

### 1. Generate RSA Keys

```bash
# Generate private key
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out jwt_private_key.pem

# Extract public key
openssl rsa -in jwt_private_key.pem -pubout -out jwt_public_key.pem

# Convert to env format
python3 - << 'PY'
from pathlib import Path
def esc(p):
    return Path(p).read_text().strip().replace("\n","\\n")
print("JWT_PRIVATE_KEY_PEM=" + esc("jwt_private_key.pem"))
print("JWT_PUBLIC_KEY_PEM=" + esc("jwt_public_key.pem"))
PY
```

### 2. Create .env File

Copy `.env.example` to `.env` and add the JWT keys from step 1.

### 3. Start Dependencies

```bash
docker compose up -d postgres nats
```

### 4. Run Service

```bash
export SCHEMA_DIR=src/events/schemas
cargo run
```

## Testing

### Health Checks

```bash
curl http://localhost:8080/health/live
curl http://localhost:8080/health/ready
```

### Auth Flow

```bash
TENANT_ID=$(uuidgen)
USER_ID=$(uuidgen)

# Register
curl -X POST http://localhost:8080/api/auth/register \
  -H "Content-Type: application/json" \
  -d "{\"tenant_id\":\"$TENANT_ID\",\"user_id\":\"$USER_ID\",\"email\":\"test@example.com\",\"password\":\"TestPassword123!\"}"

# Login
RESPONSE=$(curl -X POST http://localhost:8080/api/auth/login \
  -H "Content-Type: application/json" \
  -d "{\"tenant_id\":\"$TENANT_ID\",\"email\":\"test@example.com\",\"password\":\"TestPassword123!\"}")

ACCESS_TOKEN=$(echo $RESPONSE | jq -r '.access_token')
REFRESH_TOKEN=$(echo $RESPONSE | jq -r '.refresh_token')

# Refresh
curl -X POST http://localhost:8080/api/auth/refresh \
  -H "Content-Type: application/json" \
  -d "{\"tenant_id\":\"$TENANT_ID\",\"refresh_token\":\"$REFRESH_TOKEN\"}"

# Logout
curl -X POST http://localhost:8080/api/auth/logout \
  -H "Content-Type: application/json" \
  -d "{\"tenant_id\":\"$TENANT_ID\",\"refresh_token\":\"$REFRESH_TOKEN\"}"
```

## Events Published

- `auth.user.registered/v1` → auth.events.user.registered
- `auth.user.logged_in/v1` → auth.events.user.logged_in
- `auth.token.refreshed/v1` → auth.events.token.refreshed
- `auth.user.logged_out/v1` → auth.events.user.logged_out

All events validated against JSON schemas at publish time.

## Architecture

```
auth-rs/
├── Cargo.toml                      # Dependencies
├── .env.example                    # Environment template
├── VERSION                         # Module version
├── CHANGELOG.md                    # Change history
├── README.md                       # This file
├── api/                            # API contract reference
│   └── README.md                  # Links to /contracts/auth/
├── db/
│   └── migrations/                # Database schema
│       └── V001__init.sql
├── deploy/                         # Deployment artifacts
│   ├── Dockerfile                 # Container build
│   ├── docker-compose.yml         # Local dev stack
│   ├── docker-compose.multi.yml   # Multi-instance stack
│   └── nginx/                     # Load balancer config
├── docs/                           # Operational documentation
│   ├── DEPLOYMENT.md
│   ├── KEY-CUSTODY.md
│   ├── RUNBOOK.md
│   ├── SLO.md
│   ├── THREAT-MODEL.md
│   └── ...
├── observability/                  # Metrics and alerts
│   ├── alerts/
│   └── dashboards/
├── tests/                          # Test scripts
└── src/                            # Rust implementation
    ├── main.rs                     # Entry point
    ├── config.rs                   # Environment config
    ├── db.rs                       # Database connection
    ├── auth/                       # Auth business logic
    │   ├── password.rs            # Argon2id hashing
    │   ├── jwt.rs                 # RS256 signing
    │   ├── refresh.rs             # Token generation
    │   └── handlers.rs            # HTTP handlers
    ├── events/                     # Event system
    │   ├── envelope.rs            # Standard envelope
    │   ├── validate.rs            # Schema validation
    │   ├── publisher.rs           # NATS publishing
    │   └── schemas/               # JSON schemas
    ├── routes/                     # HTTP routing
    │   ├── health.rs              # Health endpoints
    │   └── auth.rs                # Auth endpoints
    └── middleware/
        └── tracing.rs             # Trace ID propagation
```

## Production Deployment

1. Build Docker image: `docker build -f deploy/Dockerfile -t auth-rs:1.4.0 .`
2. Set environment variables via secrets
3. Ensure PostgreSQL and NATS are accessible
4. Monitor `/health/ready` for readiness
5. Route traffic through API gateway (Traefik)


## Status

✅ Production-ready v1.0.0
