# Docker Setup - 7D Solutions Platform

## Project Structure

```
7d-infrastructure (docker-compose.infrastructure.yml)
├─ 7d-nats (nats:2.10-alpine) :4222 :8222
├─ 7d-auth-postgres (postgres:16-alpine) :5433
├─ 7d-ar-postgres (postgres:16-alpine) :5434
├─ 7d-subscriptions-postgres (postgres:16-alpine) :5435
├─ 7d-payments-postgres (postgres:16-alpine) :5436
└─ 7d-notifications-postgres (postgres:16-alpine) :5437

7d-platform (docker-compose.platform.yml)
├─ 7d-auth-1 (platform/identity-auth)
├─ 7d-auth-2 (platform/identity-auth)
└─ 7d-auth-lb (nginx:alpine) :8080

7d-modules (docker-compose.modules.yml)
├─ 7d-ar (modules/ar) :8086
├─ 7d-subscriptions (modules/subscriptions) :8087
├─ 7d-payments (modules/payments) :8088
└─ 7d-notifications (modules/notifications) :8089
```

## Labels

All containers have these labels:

```yaml
com.7dsolutions.project: "platform"
com.7dsolutions.tier: "infrastructure" | "platform" | "module"
com.7dsolutions.component: "nats" | "auth" | "ar" | "subscriptions" | "payments" | "notifications" | "*-db" | "auth-lb"
com.7dsolutions.type: "message-bus" | "database" | "api" | "proxy"
com.7dsolutions.env: "dev"
com.docker.compose.project: "7d-infrastructure" | "7d-platform" | "7d-modules"
```

## Filter Queries

```bash
# All infrastructure
docker ps --filter "label=com.7dsolutions.tier=infrastructure"

# All databases
docker ps --filter "label=com.7dsolutions.type=database"

# All modules
docker ps --filter "label=com.7dsolutions.tier=module"

# Specific component
docker ps --filter "label=com.7dsolutions.component=ar"

# By project
docker ps --filter "label=com.docker.compose.project=7d-infrastructure"
```

## Commands

### Start All
```bash
docker compose -f docker-compose.infrastructure.yml up -d
docker compose -f docker-compose.platform.yml up -d
docker compose -f docker-compose.modules.yml up -d
```

### Stop All
```bash
docker compose -f docker-compose.modules.yml stop
docker compose -f docker-compose.platform.yml stop
docker compose -f docker-compose.infrastructure.yml stop
```

### Rebuild Service
```bash
# Rebuild specific module
docker compose -f docker-compose.modules.yml build ar
docker compose -f docker-compose.modules.yml up -d ar

# Rebuild platform
docker compose -f docker-compose.platform.yml build
docker compose -f docker-compose.platform.yml up -d
```

### Logs
```bash
# All modules
docker compose -f docker-compose.modules.yml logs -f

# Specific service
docker compose -f docker-compose.modules.yml logs -f ar

# Infrastructure
docker compose -f docker-compose.infrastructure.yml logs -f nats
```

## Database Access

```bash
# AR
psql -h localhost -p 5434 -U ar_user -d ar_db

# Auth
psql -h localhost -p 5433 -U auth_user -d auth_db

# Subscriptions
psql -h localhost -p 5435 -U subscriptions_user -d subscriptions_db

# Payments
psql -h localhost -p 5436 -U payments_user -d payments_db

# Notifications
psql -h localhost -p 5437 -U notifications_user -d notifications_db
```

## NATS Access

```bash
# Monitoring UI
open http://localhost:8222

# Subscribe to all events
nats sub ">"

# Subscribe to specific module events
nats sub "ar.>"
nats sub "payments.>"
```

## Environment Setup

Required file: `.env` (copy from `.env.docker.example`)

Required variables:
- `JWT_PRIVATE_KEY_PEM`
- `JWT_PUBLIC_KEY_PEM`
- `JWT_KID`
- `BOOTSTRAP_TOKEN`
- Database credentials (optional, have defaults)

## Network

All containers use network: `7d-platform` (external)

Create if missing:
```bash
docker network create 7d-platform
```

## Dependencies

Start order (important):
1. Infrastructure (databases + NATS)
2. Platform (auth services)
3. Modules (business logic)

Cross-project dependencies removed - services find each other by container name on shared network.

## Troubleshooting

### Container won't start
```bash
docker compose -f docker-compose.<tier>.yml logs <service>
```

### Database not ready
```bash
docker ps --filter "label=com.7dsolutions.type=database" --format "{{.Names}}: {{.Status}}"
```

### Network issues
```bash
docker network inspect 7d-platform
```

### Clean restart
```bash
docker compose -f docker-compose.modules.yml down
docker compose -f docker-compose.platform.yml down
docker compose -f docker-compose.infrastructure.yml down
docker compose -f docker-compose.infrastructure.yml up -d
docker compose -f docker-compose.platform.yml up -d
docker compose -f docker-compose.modules.yml up -d
```
