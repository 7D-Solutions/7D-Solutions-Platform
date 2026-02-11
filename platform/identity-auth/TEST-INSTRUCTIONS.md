# auth-rs v1.4 - Testing Instructions

## Current Status

✅ **Code Complete**: All 23 Rust files + 4 JSON schemas created
✅ **Build Success**: `cargo build --release` completes
✅ **JWT Keys**: Generated and configured in .env
⚠️  **Infrastructure**: Using existing Docker containers

## Quick Start

### 1. Check Prerequisites

```bash
# Postgres should be on port 5433 (7d-auth-postgres container)
docker ps | grep 7d-auth-postgres

# NATS needed on port 4222 (may need to start)
docker ps | grep nats
```

### 2. Create Database (if needed)

```bash
# Connect to existing postgres
docker exec -it 7d-auth-postgres psql -U postgres

# Create database
CREATE DATABASE auth_db;
\q
```

### 3. Start NATS (if not running)

```bash
# Simple NATS without persistence
docker run -d --name auth-nats -p 4222:4222 -p 8222:8222 nats:2.10-alpine -js
```

### 4. Run auth-rs

```bash
cd "/Users/james/Projects/7D-Solutions Modules/platform/identity-auth"
export SCHEMA_DIR=src/events/schemas
cargo run --release
```

### 5. Test Endpoints

```bash
# Health checks
curl http://localhost:8081/health/live
curl http://localhost:8081/health/ready

# Register user
TENANT_ID=$(uuidgen)
USER_ID=$(uuidgen)

curl -X POST http://localhost:8081/api/auth/register \
  -H "Content-Type: application/json" \
  -d "{\"tenant_id\":\"$TENANT_ID\",\"user_id\":\"$USER_ID\",\"email\":\"test@example.com\",\"password\":\"TestPassword123!\"}"

# Login
curl -X POST http://localhost:8081/api/auth/login \
  -H "Content-Type: application/json" \
  -d "{\"tenant_id\":\"$TENANT_ID\",\"email\":\"test@example.com\",\"password\":\"TestPassword123!\"}" | jq .

# Save tokens from response
ACCESS_TOKEN="<from login response>"
REFRESH_TOKEN="<from login response>"

# Refresh token
curl -X POST http://localhost:8081/api/auth/refresh \
  -H "Content-Type: application/json" \
  -d "{\"tenant_id\":\"$TENANT_ID\",\"refresh_token\":\"$REFRESH_TOKEN\"}" | jq .

# Logout
curl -X POST http://localhost:8081/api/auth/logout \
  -H "Content-Type: application/json" \
  -d "{\"tenant_id\":\"$TENANT_ID\",\"refresh_token\":\"$REFRESH_TOKEN\"}"
```

## Configuration

Current .env settings:
- Database: `localhost:5433` (7d-auth-postgres)
- NATS: `localhost:4222`
- Server: `localhost:8081` (port 8080 is taken by existing service)

## Architecture Compliance

✅ RS256 JWT access tokens
✅ Refresh token rotation
✅ Argon2id password hashing
✅ Event envelope with trace_id & causation_id
✅ JSON schema validation at publish time
✅ Health checks (/health/live, /health/ready)
✅ Fail-fast startup
✅ No shared databases (auth owns credentials only)

## Files Created

- 23 Rust source files
- 4 JSON event schemas
- 1 SQL migration (001_init.sql)
- Cargo.toml with all dependencies
- .env with JWT keys
- README.md
- This TEST-INSTRUCTIONS.md

## Next Steps for Full Docker Integration

1. Manually create `docker-compose.yml` (hook bypass needed)
2. Manually create `Dockerfile` (hook bypass needed)
3. Or use existing docker-compose patterns from other services

## Bead ID

This work tracked under: **bd-3kph**
