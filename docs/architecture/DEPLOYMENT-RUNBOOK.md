# Deployment & Rollback Runbook

**Version:** 1.0
**Status:** Active
**Last Updated:** 2026-02-12

## Overview

This runbook provides step-by-step instructions for deploying, verifying, and rolling back the 7D Solutions Platform. It covers Docker-based deployments for development and production environments.

---

## Table of Contents

1. [Prerequisites](#prerequisites)
2. [Environment Setup](#environment-setup)
3. [Infrastructure Deployment](#infrastructure-deployment)
4. [Platform Deployment](#platform-deployment)
5. [Module Deployment](#module-deployment)
6. [Health Checks](#health-checks)
7. [Smoke Tests](#smoke-tests)
8. [Monitoring & Logs](#monitoring--logs)
9. [DLQ Inspection](#dlq-inspection)
10. [Rollback Procedures](#rollback-procedures)
11. [Troubleshooting](#troubleshooting)

---

## Prerequisites

### Software Requirements

```bash
# Docker & Docker Compose
docker --version    # Docker 24.0+
docker compose version  # Docker Compose v2.20+

# Rust (for building modules)
rustc --version     # Rust 1.75+
cargo --version

# Database client (optional, for debugging)
psql --version      # PostgreSQL 16+
```

### Network Requirements

- **External Ports:**
  - `4222` - NATS (message bus)
  - `8222` - NATS monitoring
  - `8080` - Auth service (load balanced)
  - `8086` - AR module
  - `8087` - Subscriptions module
  - `8088` - Payments module
  - `8089` - Notifications module

- **Internal Ports:**
  - `5433` - Auth PostgreSQL
  - `5434` - AR PostgreSQL
  - `5435` - Subscriptions PostgreSQL
  - `5436` - Payments PostgreSQL
  - `5437` - Notifications PostgreSQL

---

## Environment Setup

### 1. Create External Resources

```bash
# Create Docker network
docker network create 7d-platform

# Create persistent volumes
docker volume create 7d-nats-data
docker volume create 7d-auth-pgdata
docker volume create 7d-ar-pgdata
docker volume create 7d-subscriptions-pgdata
docker volume create 7d-payments-pgdata
docker volume create 7d-notifications-pgdata
```

### 2. Configure Environment Variables

Create `.env` file in project root:

```bash
# JWT signing keys for auth-rs
JWT_KID=auth-key-1
JWT_PRIVATE_KEY_PEM="-----BEGIN PRIVATE KEY-----
MIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQCuBsdZmlGizaxw
SBMxjLHiAVMgP2b/vB2kn/6eJe72KWNCJN7maHHTeUknrUOgmesz8M0Jkbo2rwPz
IW2wDHMs5n1k3SD4kT7xC4YOJmN7rKXGSReL4aGoWuQeNnJyGvCmH99DE2/4lX0T
8KOimSWUU2LJ0hZhAY8cuI0E49UQud1wKD0YsABtgTWCXf/JNjLfDaDhmL9HedAh
WDsRN/lSvEV+WqEoBwwwj0WQz5tjUtIGRtPipy2GH+62x1x3vKMyU2bNWGh6+oG+
GFUqSY0wGZyUKCJg/uLoPo+5OeWcy24slx5uDPCUNE7oSBQniGUgoKc1TxmoPNw1
p5F1iw+3AgMBAAECggEAERZGTZnK3owznGS0nxQG7+ocYi6yK3WqMsdThe4IkVqi
oynL9F3t9eB979Y3LSuKaLOslijHsBCCzUULfZKDXosVEH8WeoGHEhOFFUTsq1sv
fbwggpIOwDaIbnT/ILgHcprviNOoQLAA3iEtwqHR4EdIYbOYkewdbqmqOFWHI+31
Ai4roeIc9VZ2+q/Q3maYnSup3IDe12V5Pw0ncT6RFVnRyXaX68eXEe8/hqmsjdBD
mVzmZBC/msdODdb9QT4g0s6moecu8qUBHUsnqCthjS+vlZPf/MMr5RAhs1iDdEAP
boBSE3Be7fJIXZXril2k5nKzckgqxZ6Agf5jKkJ1pQKBgQDoiYVCwMc3wUtKwiae
be1ZpqLWDiQ+f6u6UiD5OfIHCv6CRUdzjxT2bjnpGHM0Xsk4B264b3/JIgvgLKKf
aoD+Tao1ut95nQ92vQfh9LHOm31+JJqC/ZxkDdIY5CL+ncaKjxSKro8HXjbbtwc7
DFmeuIsGJObEfKJaV87M3NWzkwKBgQC/lepdEF5uDfYvPRrTqm9nnV0BjFOHWqIe
aIaBkJdFSq62WuHiZgJUSoawCfQwB+Sn97rG1qB8fEHNGnMfFQq1M0tAhqCn+sUK
GpMbiJfDHqmMXeyTl/V7iQTe7T4mae4SnEDkPVKLrIlNj/4vk9NLrXMXYy+Q8LMp
TwqhVfyRzQKBgQCf0wVYoA9M7vnE5DSO55ce6z04Snf2zOFHKnOnWIBU/uV2vA8k
Cc+qoJAE+d0UvaEndVRQR7JYl6H57jPHxffq0Y6PZ2V5vM2IGtx0HS6oho52SMo6
Bf2bdzRUD1lODzsKuNSxjNCZi9PAp8e8efyO7t/+1RYXLmKYHYnxnEb1KQKBgQC0
K4f7fSlQ1lBunEheRin+hz6v9geXguRzNFlJ/3BC+bjURSOohcYq/usrIjFB+ipO
y+oalDzY1QIMoJMi5+bqARMD25e6YVpr5hHyEsKl/G/2UV3qbz2sr26lNvb7qSL6
3XcpLYIzWE7HYmLo21waD0Ps+poA9FuIvYyBrRuZAQKBgQC5InrXyU9CPpbSuAFk
7KwA/2VjJm8mw2lD4OJUckM7GFGZY5UGCJ2A52MOhb0/bYFKlV9oPhnmIjMdW9gx
vxIUOy2kudHsnbFzWBb3SNW5qpL8RuOQlsgbUxxCjF5sVoyZRa4ZJdbbyFAMgCd+
hVVG9aCctsAdws7WdKVTzxNAMA==
-----END PRIVATE KEY-----"

JWT_PUBLIC_KEY_PEM="-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEArgbHWZpRos2scEgTMYyx
4gFTID9m/7wdpJ/+niXu9iljQiTe5mhx03lJJ61DoJnrM/DNCZG6Nq8D8yFtsAxz
LOZ9ZN0g+JE+8QuGDiZje6ylxkkXi+GhqFrkHjZychrwph/fQxNv+JV9E/Cjopkl
lFNiydIWYQGPHLiNBOPVELndcCg9GLAAbYE1gl3/yTYy3w2g4Zi/R3nQIVg7ETf5
UrxFflqhKAcMMI9FkM+bY1LSBkbT4qcthh/utsdcd7yjMlNmzVhoevqBvhhVKkmN
MBmclCgiYP7i6D6PuTnlnMtuLJcebgzwlDRO6EgUJ4hlIKCnNU8ZqDzcNaeRdYsP
twIDAQAB
-----END PUBLIC KEY-----"

# Bootstrap token for initial tenant creation
BOOTSTRAP_TOKEN=smoke-test-bootstrap-token-minimum-32-characters-required-for-security

# Event bus configuration
BUS_TYPE=nats
NATS_URL=nats://7d-nats:4222

# Database credentials (defaults shown, override as needed)
AUTH_POSTGRES_DB=auth_db
AUTH_POSTGRES_USER=auth_user
AUTH_POSTGRES_PASSWORD=auth_pass

AR_POSTGRES_DB=ar_db
AR_POSTGRES_USER=ar_user
AR_POSTGRES_PASSWORD=ar_pass

SUBSCRIPTIONS_POSTGRES_DB=subscriptions_db
SUBSCRIPTIONS_POSTGRES_USER=subscriptions_user
SUBSCRIPTIONS_POSTGRES_PASSWORD=subscriptions_pass

PAYMENTS_POSTGRES_DB=payments_db
PAYMENTS_POSTGRES_USER=payments_user
PAYMENTS_POSTGRES_PASSWORD=payments_pass

NOTIFICATIONS_POSTGRES_DB=notifications_db
NOTIFICATIONS_POSTGRES_USER=notifications_user
NOTIFICATIONS_POSTGRES_PASSWORD=notifications_pass

# Logging
RUST_LOG=info
```

**Production Notes:**
- Generate unique JWT keys using: `openssl genpkey -algorithm RSA -out private.pem -pkeyopt rsa_keygen_bits:2048`
- Use secrets management (AWS Secrets Manager, HashiCorp Vault, etc.)
- Never commit `.env` to version control

---

## Infrastructure Deployment

### Start Infrastructure Services

Infrastructure includes NATS message bus and PostgreSQL databases.

```bash
# Start all infrastructure services
docker compose -f docker-compose.infrastructure.yml up -d

# Verify all services are running
docker compose -f docker-compose.infrastructure.yml ps
```

**Expected Output:**
```
NAME                      STATUS              PORTS
7d-nats                   Up 10s (healthy)    0.0.0.0:4222->4222/tcp, 0.0.0.0:8222->8222/tcp
7d-auth-postgres          Up 10s (healthy)    0.0.0.0:5433->5432/tcp
7d-ar-postgres            Up 10s (healthy)    0.0.0.0:5434->5432/tcp
7d-subscriptions-postgres Up 10s (healthy)    0.0.0.0:5435->5432/tcp
7d-payments-postgres      Up 10s (healthy)    0.0.0.0:5436->5432/tcp
7d-notifications-postgres Up 10s (healthy)    0.0.0.0:5437->5432/tcp
```

### Verify Infrastructure Health

```bash
# Check NATS
curl http://localhost:8222/healthz
# Expected: OK

# Check NATS JetStream
docker logs 7d-nats 2>&1 | grep -i jetstream
# Expected: "Starting JetStream"

# Check databases
for port in 5433 5434 5435 5436 5437; do
  echo "Testing port $port..."
  docker exec -i 7d-nats sh -c "nc -zv host.docker.internal $port" 2>&1 | grep succeeded || echo "Failed"
done
```

---

## Platform Deployment

### Build Platform Services

```bash
# Build Auth service containers
docker compose -f docker-compose.platform.yml build

# Start platform services
docker compose -f docker-compose.platform.yml up -d

# Verify
docker compose -f docker-compose.platform.yml ps
```

**Expected Output:**
```
NAME          STATUS              PORTS
7d-auth-1     Up 30s
7d-auth-2     Up 30s
7d-auth-lb    Up 30s              0.0.0.0:8080->80/tcp
```

### Verify Platform Health

```bash
# Check auth service through load balancer
curl http://localhost:8080/api/health
# Expected: {"status":"ok"}

# Verify both auth instances
docker logs 7d-auth-1 2>&1 | grep "Server listening"
docker logs 7d-auth-2 2>&1 | grep "Server listening"
```

---

## Module Deployment

### Build Module Services

```bash
# Build all module containers (AR, Subscriptions, Payments, Notifications)
docker compose -f docker-compose.modules.yml build

# Start module services
docker compose -f docker-compose.modules.yml up -d

# Monitor startup logs
docker compose -f docker-compose.modules.yml logs -f
```

### Verify Module Health

Wait for all services to show `(healthy)` status:

```bash
# Check all services
docker compose -f docker-compose.modules.yml ps

# Individual health checks
curl http://localhost:8086/api/health  # AR
curl http://localhost:8087/api/health  # Subscriptions
curl http://localhost:8088/api/health  # Payments
curl http://localhost:8089/api/health  # Notifications
```

**Expected Response (all modules):**
```json
{"status":"ok"}
```

### Verify Event Consumers

Check that each module has subscribed to its event streams:

```bash
# Check AR consumer
docker logs 7d-ar 2>&1 | grep "Subscribed to"
# Expected: "Subscribed to payments.events.payments.payment.succeeded"

# Check Payments consumer
docker logs 7d-payments 2>&1 | grep "Subscribed to"
# Expected: "Subscribed to ar.events.ar.payment.collection.requested"

# Check Notifications consumers (3 consumers)
docker logs 7d-notifications 2>&1 | grep "Subscribed to"
# Expected: Multiple subscription confirmations for different event types
```

---

## Health Checks

### Quick Health Check

```bash
#!/bin/bash
# Save as: scripts/health-check.sh

echo "🏥 Health Check - 7D Solutions Platform"
echo "========================================"

# Infrastructure
echo ""
echo "📦 Infrastructure:"
curl -sf http://localhost:8222/healthz > /dev/null && echo "✅ NATS" || echo "❌ NATS"

for port in 5433:Auth 5434:AR 5435:Subscriptions 5436:Payments 5437:Notifications; do
  p=${port%:*}
  name=${port#*:}
  nc -z localhost $p 2>/dev/null && echo "✅ $name DB" || echo "❌ $name DB"
done

# Platform
echo ""
echo "🔐 Platform:"
curl -sf http://localhost:8080/api/health > /dev/null && echo "✅ Auth (LB)" || echo "❌ Auth (LB)"

# Modules
echo ""
echo "🧩 Modules:"
curl -sf http://localhost:8086/api/health > /dev/null && echo "✅ AR" || echo "❌ AR"
curl -sf http://localhost:8087/api/health > /dev/null && echo "✅ Subscriptions" || echo "❌ Subscriptions"
curl -sf http://localhost:8088/api/health > /dev/null && echo "✅ Payments" || echo "❌ Payments"
curl -sf http://localhost:8089/api/health > /dev/null && echo "✅ Notifications" || echo "❌ Notifications"

echo ""
echo "========================================"
```

### Detailed Health Check

```bash
# Check container resource usage
docker stats --no-stream

# Check container restart counts
docker compose -f docker-compose.yml ps -a --format "table {{.Name}}\t{{.Status}}"

# Check database connections
docker exec -i 7d-ar-postgres psql -U ar_user -d ar_db -c "SELECT count(*) FROM pg_stat_activity;"
```

---

## Smoke Tests

### Happy Path - End-to-End Flow

This test verifies the complete billing cycle: subscription → invoice → payment → notification.

```bash
# Run E2E happy path test
cd e2e-tests
cargo test --test real_e2e -- --test-threads=1 --nocapture

# Keep containers running for debugging (optional)
E2E_KEEP_CONTAINERS=1 cargo test --test real_e2e -- --test-threads=1 --nocapture
```

**What the test verifies:**
1. ✅ Create tenant and customer
2. ✅ Create subscription with recurring billing
3. ✅ Trigger bill run
4. ✅ AR creates invoice
5. ✅ AR publishes `ar.payment.collection.requested` event
6. ✅ Payments processes payment
7. ✅ Payments publishes `payments.payment.succeeded` event
8. ✅ AR marks invoice as `paid`
9. ✅ Notifications processes events
10. ✅ All DLQ tables are empty (no errors)

### Sad Path - Payment Failure

This test verifies error handling when payment processing fails.

```bash
# Run E2E sad path test
cd e2e-tests
cargo test --test real_e2e_sad_path -- --test-threads=1 --nocapture
```

**What the test verifies:**
1. ✅ Create tenant and customer with failing payment method
2. ✅ Create subscription and trigger bill run
3. ✅ AR creates invoice (status: `open`)
4. ✅ Payments attempts payment and fails
5. ✅ Payments publishes `payments.payment.failed` event
6. ✅ AR receives failure event
7. ✅ Invoice remains `open` (not paid)
8. ✅ Notifications processes failure event
9. ✅ DLQ tables are empty (expected failures don't go to DLQ)
10. ✅ No `payment.succeeded` event emitted

### Manual Smoke Test

For quick verification without running full test suite:

```bash
#!/bin/bash
# Save as: scripts/smoke-test-manual.sh

TENANT_ID="tenant-$(uuidgen | tr '[:upper:]' '[:lower:]')"
API_KEY="test-api-key-123"

echo "🔍 Manual Smoke Test"
echo "Tenant: $TENANT_ID"
echo ""

# 1. Create customer in AR
echo "1️⃣ Creating customer..."
CUSTOMER_ID=$(curl -s -X POST http://localhost:8086/api/customers \
  -H "X-Tenant-ID: $TENANT_ID" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Smoke Test Customer",
    "email": "smoke@test.com"
  }' | jq -r '.id')

echo "   Customer ID: $CUSTOMER_ID"

# 2. Create invoice
echo "2️⃣ Creating invoice..."
INVOICE_ID=$(curl -s -X POST http://localhost:8086/api/invoices \
  -H "X-Tenant-ID: $TENANT_ID" \
  -H "Content-Type: application/json" \
  -d "{
    \"customer_id\": \"$CUSTOMER_ID\",
    \"amount_minor\": 10000,
    \"currency\": \"USD\",
    \"due_date\": \"2026-03-01\"
  }" | jq -r '.id')

echo "   Invoice ID: $INVOICE_ID"

# 3. Wait for async processing
echo "3️⃣ Waiting for event processing..."
sleep 5

# 4. Check invoice status
echo "4️⃣ Checking invoice status..."
curl -s http://localhost:8086/api/invoices/$INVOICE_ID \
  -H "X-Tenant-ID: $TENANT_ID" | jq '.status'

echo ""
echo "✅ Smoke test complete!"
```

---

## Monitoring & Logs

### View Logs

```bash
# All services
docker compose -f docker-compose.yml logs -f

# Specific service
docker logs -f 7d-ar
docker logs -f 7d-payments
docker logs -f 7d-notifications

# Filter by log level
docker logs 7d-ar 2>&1 | grep ERROR
docker logs 7d-payments 2>&1 | grep WARN

# View last 100 lines
docker logs --tail 100 7d-ar

# View logs with timestamps
docker logs --timestamps 7d-ar
```

### Search for Specific Events

```bash
# Find event processing logs
docker logs 7d-ar 2>&1 | grep "event_id"

# Find correlation ID across services
CORRELATION_ID="abc-123-def"
for service in ar payments notifications; do
  echo "=== $service ==="
  docker logs 7d-$service 2>&1 | grep "$CORRELATION_ID"
done

# Check for consumer subscriptions
docker compose -f docker-compose.modules.yml logs | grep "Subscribed to"

# Check for outbox publishing
docker compose -f docker-compose.modules.yml logs | grep "Published.*events to bus"
```

### Monitor NATS

```bash
# NATS monitoring UI
open http://localhost:8222

# Check JetStream streams
docker exec -it 7d-nats nats stream list

# Check JetStream consumers
docker exec -it 7d-nats nats consumer list

# View stream info
docker exec -it 7d-nats nats stream info AR_EVENTS
```

---

## DLQ Inspection

Dead Letter Queue (DLQ) tables capture events that failed processing after retry exhaustion.

### Check DLQ Tables

Each module has a `failed_events` table:

```bash
# AR Module DLQ
docker exec -i 7d-ar-postgres psql -U ar_user -d ar_db << 'EOF'
SELECT
  id,
  event_id,
  subject,
  tenant_id,
  substring(error, 1, 100) as error_preview,
  retry_count,
  failed_at
FROM failed_events
ORDER BY failed_at DESC
LIMIT 10;
EOF

# Payments Module DLQ
docker exec -i 7d-payments-postgres psql -U payments_user -d payments_db << 'EOF'
SELECT
  id,
  event_id,
  subject,
  tenant_id,
  substring(error, 1, 100) as error_preview,
  retry_count,
  failed_at
FROM failed_events
ORDER BY failed_at DESC
LIMIT 10;
EOF

# Notifications Module DLQ
docker exec -i 7d-notifications-postgres psql -U notifications_user -d notifications_db << 'EOF'
SELECT
  id,
  event_id,
  subject,
  tenant_id,
  substring(error, 1, 100) as error_preview,
  retry_count,
  failed_at
FROM failed_events
ORDER BY failed_at DESC
LIMIT 10;
EOF
```

### DLQ Analysis Script

```bash
#!/bin/bash
# Save as: scripts/check-dlq.sh

echo "🔍 DLQ Health Check"
echo "==================="

check_dlq() {
  local module=$1
  local db_name=$2
  local db_user=$3

  count=$(docker exec -i 7d-${module}-postgres psql -U $db_user -d $db_name -t -c \
    "SELECT COUNT(*) FROM failed_events;")

  if [ "$count" -eq 0 ]; then
    echo "✅ $module: 0 failed events"
  else
    echo "⚠️  $module: $count failed events"
    docker exec -i 7d-${module}-postgres psql -U $db_user -d $db_name << EOF
SELECT
  subject,
  COUNT(*) as count,
  MAX(failed_at) as last_failure
FROM failed_events
GROUP BY subject
ORDER BY count DESC;
EOF
  fi
}

check_dlq "ar" "ar_db" "ar_user"
check_dlq "payments" "payments_db" "payments_user"
check_dlq "notifications" "notifications_db" "notifications_user"

echo ""
echo "==================="
```

### Inspect Failed Event Details

```bash
# Get full details of a specific failed event
EVENT_ID="<event-id-from-dlq>"

docker exec -i 7d-ar-postgres psql -U ar_user -d ar_db << EOF
SELECT
  event_id,
  subject,
  tenant_id,
  envelope_json,
  error,
  retry_count,
  failed_at
FROM failed_events
WHERE event_id = '$EVENT_ID';
EOF
```

### DLQ Remediation

When DLQ contains events, investigate and fix:

1. **Analyze the error pattern:**
   ```sql
   SELECT error, COUNT(*)
   FROM failed_events
   GROUP BY error
   ORDER BY count DESC;
   ```

2. **Check if issue is resolved** (e.g., external service is back up)

3. **Replay events** (after fixing the underlying issue):
   ```sql
   -- Extract events for replay
   SELECT envelope_json FROM failed_events WHERE id IN (...);

   -- Delete from DLQ after successful replay
   DELETE FROM failed_events WHERE id IN (...);
   ```

4. **Purge obsolete failures** (after manual resolution):
   ```sql
   DELETE FROM failed_events WHERE failed_at < NOW() - INTERVAL '30 days';
   ```

---

## Rollback Procedures

### Rollback Docker Deployment

#### Option 1: Rollback to Previous Image Tag

```bash
# List available image tags
docker images | grep 7d-solutions

# Update docker-compose.yml to use previous tag
# Change: image: ghcr.io/7d-solutions/ar:2.1.0
# To:     image: ghcr.io/7d-solutions/ar:2.0.1

# Recreate services with old image
docker compose -f docker-compose.modules.yml up -d --force-recreate

# Verify rollback
docker compose -f docker-compose.modules.yml ps
./scripts/health-check.sh
```

#### Option 2: Rebuild from Git SHA

```bash
# Checkout previous version
git log --oneline -20  # Find the commit SHA
git checkout <previous-sha>

# Rebuild and restart
docker compose -f docker-compose.modules.yml build
docker compose -f docker-compose.modules.yml up -d --force-recreate

# Verify
docker compose -f docker-compose.modules.yml ps
./scripts/health-check.sh
```

### Rollback Single Module

If only one module has an issue:

```bash
# Stop problematic module
docker compose -f docker-compose.modules.yml stop ar

# Update to previous version (in docker-compose.modules.yml)
# or rebuild from previous git SHA

# Start module
docker compose -f docker-compose.modules.yml up -d ar

# Verify
curl http://localhost:8086/api/health
docker logs -f 7d-ar
```

### Database Rollback

**⚠️ WARNING:** Database rollbacks are complex and risky.

**Best Practice:** Use forward-only migrations.

If rollback is absolutely necessary:

```bash
# 1. Stop all services accessing the database
docker compose -f docker-compose.modules.yml stop ar

# 2. Backup current database
docker exec 7d-ar-postgres pg_dump -U ar_user ar_db > ar_backup_$(date +%Y%m%d_%H%M%S).sql

# 3. Apply rollback migration (if available)
docker exec -i 7d-ar-postgres psql -U ar_user -d ar_db < rollback_migration.sql

# 4. Restart service
docker compose -f docker-compose.modules.yml up -d ar

# 5. Verify
./scripts/health-check.sh
./scripts/smoke-test-manual.sh
```

### Emergency Rollback

For critical production issues:

```bash
# 1. Immediate stop
docker compose -f docker-compose.modules.yml stop

# 2. Restore from backup
docker compose -f docker-compose.infrastructure.yml down
docker volume rm 7d-ar-pgdata
docker volume create 7d-ar-pgdata
# Restore database from backup

# 3. Deploy last known good version
git checkout <last-good-sha>
docker compose -f docker-compose.yml build
docker compose -f docker-compose.yml up -d

# 4. Verify
./scripts/health-check.sh

# 5. Post-mortem (document what happened)
```

---

## Troubleshooting

### Services Won't Start

**Problem:** Containers exit immediately or stay in `Restarting` state.

```bash
# Check logs for errors
docker compose -f docker-compose.modules.yml logs

# Common issues:
# - Database not ready: Wait for health check
# - Environment variables missing: Check .env file
# - Port conflicts: Check if ports already in use (lsof -i :8086)
# - Build failures: Run build separately to see errors

# Rebuild with verbose output
docker compose -f docker-compose.modules.yml build --no-cache --progress=plain
```

### Health Checks Failing

**Problem:** Service status shows `unhealthy`.

```bash
# Check why health check is failing
docker inspect 7d-ar --format='{{json .State.Health}}' | jq

# Common issues:
# - Service not listening on expected port
# - Health endpoint not responding
# - Database connection failing

# Debug inside container
docker exec -it 7d-ar sh
curl http://localhost:8086/api/health
```

### Events Not Processing

**Problem:** Events published but not consumed.

```bash
# 1. Verify NATS is healthy
curl http://localhost:8222/healthz

# 2. Check if consumers subscribed
docker logs 7d-ar 2>&1 | grep "Subscribed to"
docker logs 7d-payments 2>&1 | grep "Subscribed to"

# 3. Check outbox publishing
docker exec -i 7d-ar-postgres psql -U ar_user -d ar_db -c \
  "SELECT COUNT(*) FROM events_outbox WHERE published_at IS NULL;"
# Should be 0 (all published)

# 4. Check for events in DLQ
./scripts/check-dlq.sh

# 5. Verify event correlation
CORRELATION_ID="<id-from-logs>"
for svc in ar payments notifications; do
  docker logs 7d-$svc 2>&1 | grep "$CORRELATION_ID"
done
```

### High Resource Usage

**Problem:** Services consuming too much CPU/memory.

```bash
# Check resource usage
docker stats

# Identify problematic container
docker inspect --format='{{.State.Pid}}' 7d-ar
top -p <pid>

# Check for memory leaks
docker exec 7d-ar sh -c 'ps aux | sort -nk 4'

# Restart service if needed
docker compose -f docker-compose.modules.yml restart ar
```

### Database Connection Issues

**Problem:** Services can't connect to database.

```bash
# Test database connectivity
docker exec -it 7d-ar sh
nc -zv 7d-ar-postgres 5432

# Check database logs
docker logs 7d-ar-postgres

# Check connection pool
docker exec -i 7d-ar-postgres psql -U ar_user -d ar_db -c \
  "SELECT * FROM pg_stat_activity WHERE datname='ar_db';"

# Check for connection limits
docker exec -i 7d-ar-postgres psql -U ar_user -d ar_db -c \
  "SHOW max_connections;"
```

---

## Quick Reference

### Start Everything

```bash
# One-command full startup
docker compose up -d

# Verify
docker compose ps
./scripts/health-check.sh
```

### Stop Everything

```bash
# Stop all services (keep volumes)
docker compose down

# Stop and remove volumes (⚠️ DATA LOSS)
docker compose down -v
```

### Restart Single Service

```bash
docker compose restart ar
docker compose logs -f ar
```

### Clean Slate

```bash
# Stop everything
docker compose down -v

# Remove dangling images
docker image prune -f

# Remove external resources
docker volume rm 7d-nats-data 7d-ar-pgdata 7d-subscriptions-pgdata 7d-payments-pgdata 7d-notifications-pgdata
docker network rm 7d-platform

# Start fresh
./docs/architecture/DEPLOYMENT-RUNBOOK.md  # Follow from top
```

---

## See Also

- [Release Policy](../governance/RELEASE-POLICY.md) - Release process and versioning
- [Operations Standard](OPERATIONS-STANDARD.md) - Logging and metrics standards
- [Troubleshooting Guide](../../TROUBLESHOOTING_RUNDOWN.md) - Detailed troubleshooting
- [Test Standard](TEST-STANDARD.md) - E2E and integration testing
