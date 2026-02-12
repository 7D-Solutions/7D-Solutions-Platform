# 7D Solutions Platform - Docker Setup

Clean, properly-labeled Docker Compose configuration for the 7D Solutions Platform.

## Project Structure

```
docker-compose.yml               # Main orchestration (includes all)
├── docker-compose.infrastructure.yml  # NATS + Databases
├── docker-compose.platform.yml        # Identity-Auth + Reference
└── docker-compose.modules.yml         # AR, Subscriptions, Payments, Notifications
```

## Label Schema

All containers are labeled with:

```yaml
com.7dsolutions.project: "platform"
com.7dsolutions.tier: "infrastructure|platform|module"
com.7dsolutions.component: "nats|auth|ar|subscriptions|payments|notifications|..."
com.7dsolutions.type: "message-bus|database|api|proxy"
com.7dsolutions.env: "dev"
```

### Filter Examples

```bash
# Show all infrastructure
docker ps --filter "label=com.7dsolutions.tier=infrastructure"

# Show all databases
docker ps --filter "label=com.7dsolutions.type=database"

# Show AR module
docker ps --filter "label=com.7dsolutions.component=ar"

# Show all modules
docker ps --filter "label=com.7dsolutions.tier=module"
```

## Services

### Infrastructure Tier (Port Range: 4xxx-5xxx)
- **nats**: Message bus (:4222, :8222)
- **auth-postgres**: Auth database (:5433)
- **reference-postgres**: Reference database (:5438)
- **ar-postgres**: AR database (:5434)
- **subscriptions-postgres**: Subscriptions database (:5435)
- **payments-postgres**: Payments database (:5436)
- **notifications-postgres**: Notifications database (:5437)

### Platform Tier (Port Range: 8xxx)
- **auth-1**: Identity-Auth instance 1
- **auth-2**: Identity-Auth instance 2
- **auth-lb**: Nginx load balancer (:8080)
- **reference**: Reference data service (:8090)

### Module Tier (Port Range: 808x)
- **ar**: Accounts Receivable (:8086)
- **subscriptions**: Subscription management (:8087)
- **payments**: Payment processing (:8088)
- **notifications**: Notification delivery (:8089)

## Quick Start

### 1. Setup Environment

```bash
# Copy environment template
cp .env.docker.example .env.docker

# Edit with your values
nano .env.docker
```

### 2. Start All Services

```bash
# Create network first
docker network create 7d-platform

# Start everything
docker compose up -d

# Or start selectively
docker compose -f docker-compose.infrastructure.yml up -d
docker compose -f docker-compose.platform.yml up -d
docker compose -f docker-compose.modules.yml up -d
```

### 3. Check Status

```bash
# All platform containers
docker ps --filter "label=com.7dsolutions.project=platform"

# View logs
docker compose logs -f

# View specific service
docker compose logs -f ar
```

### 4. Stop Services

```bash
# Stop all
docker compose stop

# Stop specific tier
docker compose -f docker-compose.modules.yml stop
```

## Development Workflow

### Rebuild After Code Changes

```bash
# Rebuild specific service
docker compose build ar
docker compose up -d ar

# Rebuild all modules
docker compose -f docker-compose.modules.yml build
docker compose -f docker-compose.modules.yml up -d
```

### Access Databases

```bash
# AR database
psql -h localhost -p 5434 -U ar_user -d ar_db

# Auth database
psql -h localhost -p 5433 -U auth_user -d auth_db
```

### View NATS

```bash
# NATS monitoring
open http://localhost:8222

# Subscribe to all events
nats sub ">"
```

## Migration from Old Setup

If you have containers from `7d-services`, `7d-solutionsmodules`, or `7d-databases`:

```bash
# Stop old containers
cd "/Users/james/Projects/7D-Solutions Modules"
docker compose -f docker-compose.yml stop
docker compose -f docker-compose.platform.yml stop
docker compose -f docker-compose.db.yml stop

# Switch to new setup
cd "/Users/james/Projects/7D-Solutions Platform"
docker network create 7d-platform
docker compose up -d
```

## Troubleshooting

### Network Issues

```bash
# Recreate network
docker network rm 7d-platform
docker network create 7d-platform
docker compose up -d
```

### Database Connection Issues

```bash
# Check database health
docker ps --filter "label=com.7dsolutions.type=database"

# View database logs
docker logs 7d-ar-postgres
```

### Service Not Starting

```bash
# Check service logs
docker compose logs ar

# Check dependencies
docker compose ps
```

## Cleaning Up

```bash
# Stop and remove containers (keeps volumes)
docker compose down

# Remove everything including volumes (DESTRUCTIVE)
docker compose down -v
```
