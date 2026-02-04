# Production Operations Guide

## Critical Rules for Billing Database

### 🚨 Rule #1: Always Pin Schema Path

**EVERY billing migration command MUST include the schema path:**

```bash
# ✅ CORRECT
npx prisma migrate deploy --schema=packages/ar/prisma/schema.prisma

# ❌ WRONG (will use main app schema!)
npx prisma migrate deploy
```

This prevents accidentally migrating the wrong database at 2am.

### 🚨 Rule #2: Separate Backup Policies

Billing database = **revenue-critical**. Different backup requirements than main app:

| Database | Priority | Backup Frequency | Retention | Recovery SLA |
|----------|----------|------------------|-----------|--------------|
| **Billing DB** | CRITICAL | Every 6 hours | 90 days | < 1 hour |
| Main App DB | HIGH | Daily | 30 days | < 4 hours |

**Why different:**
- Billing = money, subscriptions, payment history
- Losing billing data = revenue loss, compliance issues
- Main app data can often be recreated

### 🚨 Rule #3: Restricted Access

Billing database should have **stricter access controls**:

```sql
-- Billing DB: Read-only for most engineers
GRANT SELECT ON billing_db.* TO 'readonly_user'@'%';

-- Only billing service + senior engineers get write access
GRANT ALL ON billing_db.* TO 'billing_service'@'app-server';
GRANT ALL ON billing_db.* TO 'admin'@'%';
```

## Migration Workflow (Production)

### Development
```bash
cd packages/ar

# Always specify schema
npx prisma migrate dev --schema=./prisma/schema.prisma --name add_feature
```

### Staging
```bash
# Deploy migrations
npx prisma migrate deploy --schema=packages/ar/prisma/schema.prisma

# Verify
npx prisma studio --schema=packages/ar/prisma/schema.prisma
```

### Production

**Pre-deployment checklist:**
- [ ] Migrations tested in staging
- [ ] Backup taken before deployment
- [ ] Rollback plan documented
- [ ] Schema path confirmed in deploy script

```bash
#!/bin/bash
# deploy-billing-migrations.sh

set -e

SCHEMA_PATH="packages/ar/prisma/schema.prisma"

echo "Taking pre-migration backup..."
./scripts/backup-billing-db.sh

echo "Deploying billing migrations..."
npx prisma migrate deploy --schema=$SCHEMA_PATH

echo "Verifying migration status..."
npx prisma migrate status --schema=$SCHEMA_PATH

echo "Billing migrations deployed successfully"
```

## Backup & Restore

### Backup Script

```bash
#!/bin/bash
# backup-billing-db.sh

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
BACKUP_DIR="/backups/billing"
DB_NAME="billing_db"

# Extract connection details from DATABASE_URL_BILLING
# mysql://user:pass@host:3306/billing_db

mysqldump \
  --single-transaction \
  --routines \
  --triggers \
  --events \
  $DB_NAME > $BACKUP_DIR/billing_$TIMESTAMP.sql

# Compress
gzip $BACKUP_DIR/billing_$TIMESTAMP.sql

# Upload to S3 (or your backup storage)
aws s3 cp $BACKUP_DIR/billing_$TIMESTAMP.sql.gz \
  s3://backups/billing/$TIMESTAMP.sql.gz

echo "Billing backup complete: $TIMESTAMP"
```

### Restore Script

```bash
#!/bin/bash
# restore-billing-db.sh

BACKUP_FILE=$1

if [ -z "$BACKUP_FILE" ]; then
  echo "Usage: ./restore-billing-db.sh <backup_file.sql.gz>"
  exit 1
fi

echo "⚠️  WARNING: This will OVERWRITE billing database"
read -p "Are you sure? (yes/no): " confirm

if [ "$confirm" != "yes" ]; then
  echo "Restore cancelled"
  exit 0
fi

# Decompress
gunzip -c $BACKUP_FILE > /tmp/billing_restore.sql

# Restore
mysql billing_db < /tmp/billing_restore.sql

# Clean up
rm /tmp/billing_restore.sql

echo "Billing database restored from $BACKUP_FILE"
```

### Automated Backup Schedule (cron)

```cron
# Billing database backups (every 6 hours)
0 */6 * * * /opt/scripts/backup-billing-db.sh

# Main app database backups (daily at 2am)
0 2 * * * /opt/scripts/backup-main-db.sh
```

## Monitoring

### Key Metrics to Track

```javascript
// Add to your monitoring service (Datadog, New Relic, etc.)

// 1. Database connection pool
billingPrisma.$on('query', (e) => {
  metrics.histogram('billing.query.duration', e.duration);
  metrics.increment('billing.query.count');
});

// 2. Webhook processing
metrics.increment('billing.webhook.received', { app_id, event_type });
metrics.increment('billing.webhook.processed', { status });

// 3. Subscription changes
metrics.increment('billing.subscription.created', { app_id, plan_id });
metrics.increment('billing.subscription.canceled', { app_id, plan_id });

// 4. Database size
# Weekly cron job
SELECT
  table_name,
  ROUND(((data_length + index_length) / 1024 / 1024), 2) AS size_mb
FROM information_schema.tables
WHERE table_schema = 'billing_db'
ORDER BY (data_length + index_length) DESC;
```

### Alert Thresholds

| Metric | Warning | Critical | Action |
|--------|---------|----------|--------|
| Webhook failures | > 5% | > 10% | Check Tilled status, signature secrets |
| Query latency | > 500ms | > 1s | Check indexes, connection pool |
| DB size | > 80% disk | > 90% disk | Scale storage |
| Failed subscriptions | > 2% | > 5% | Check payment methods, dunning |

## Disaster Recovery

### Scenario 1: Billing DB Corruption

```bash
# 1. Stop billing service
systemctl stop billing-service

# 2. Restore from latest backup
./restore-billing-db.sh /backups/billing/latest.sql.gz

# 3. Verify data integrity
npx prisma studio --schema=packages/ar/prisma/schema.prisma

# 4. Restart service
systemctl start billing-service

# 5. Monitor webhooks for replay
# Check billing_webhooks table for gaps in received_at
```

### Scenario 2: Accidental Data Deletion

```bash
# DO NOT PANIC
# 1. Immediately take snapshot of current state
mysqldump billing_db > /tmp/post-incident.sql

# 2. Identify time of deletion (from audit logs)
# 3. Restore from backup BEFORE deletion
# 4. Extract only deleted records
# 5. Merge with current state

# Example: Restore deleted subscriptions
SELECT * FROM billing_subscriptions
WHERE created_at < '2026-01-22'
AND id NOT IN (SELECT id FROM current.billing_subscriptions);
```

### Scenario 3: Database Split (Moving to Separate Server)

```bash
# 1. Create new billing database server
# 2. Take backup from current location
mysqldump billing_db > billing_migration.sql

# 3. Restore to new server
mysql -h new-billing-server billing_db < billing_migration.sql

# 4. Update DATABASE_URL_BILLING in production
DATABASE_URL_BILLING="mysql://user:pass@new-billing-server:3306/billing_db"

# 5. Deploy with new connection string
# 6. Verify connectivity
npx prisma migrate status --schema=packages/ar/prisma/schema.prisma

# 7. Cut over traffic (zero-downtime)
# 8. Monitor for errors
# 9. Delete old billing data after 7-day verification period
```

## Security Checklist

### Database Level
- [ ] Billing DB has separate user credentials
- [ ] SSL/TLS enforced for connections
- [ ] IP whitelist configured
- [ ] Read-only replicas for analytics
- [ ] Audit logging enabled

### Application Level
- [ ] `DATABASE_URL_BILLING` stored in secrets manager (not .env file in prod)
- [ ] Connection string rotated every 90 days
- [ ] Prisma logging configured for production
- [ ] Query timeout set (prevent long-running queries)

### Compliance
- [ ] PCI DSS: No raw card data in database (verified)
- [ ] GDPR: Customer data deletion procedure documented
- [ ] SOC 2: Access logs retained for audit
- [ ] Data residency: Billing DB in correct region

## Performance Optimization

### Connection Pooling

```javascript
// packages/ar/backend/src/prisma.js
const billingPrisma = new PrismaClient({
  datasources: {
    db: {
      url: process.env.DATABASE_URL_BILLING
    }
  },
  log: process.env.NODE_ENV === 'production' ? ['error', 'warn'] : ['query', 'error', 'warn'],
  errorFormat: 'minimal',
  // Connection pool settings
  pool: {
    min: 2,
    max: 10,
    acquireTimeoutMillis: 30000,
    idleTimeoutMillis: 60000
  }
});
```

### Indexes Verified

```sql
-- Run monthly to ensure indexes are used
EXPLAIN SELECT * FROM billing_subscriptions
WHERE app_id = 'trashtech' AND status = 'active';

-- Should use idx_app_id or idx_status
-- If "Using filesort" appears, add composite index
```

### Query Optimization

```javascript
// ❌ BAD: N+1 queries
const subscriptions = await billingPrisma.billing_subscriptions.findMany();
for (const sub of subscriptions) {
  const customer = await billingPrisma.billing_customers.findUnique({
    where: { id: sub.billing_customer_id }
  });
}

// ✅ GOOD: Single query with join
const subscriptions = await billingPrisma.billing_subscriptions.findMany({
  include: {
    billing_customers: true
  }
});
```

## Runbook: Common Issues

### Issue: "Can't connect to billing database"

**Symptoms:** 500 errors on billing endpoints
**Check:**
```bash
# 1. Verify DATABASE_URL_BILLING is set
echo $DATABASE_URL_BILLING

# 2. Test connection directly
mysql -h billing-db-host -u user -p billing_db

# 3. Check Prisma can connect
npx prisma db pull --schema=packages/ar/prisma/schema.prisma
```

**Fix:** Update connection string, restart service

---

### Issue: "Webhook processing slow"

**Symptoms:** `billing_webhooks.status = 'received'` piling up
**Check:**
```sql
SELECT status, COUNT(*)
FROM billing_webhooks
WHERE received_at > NOW() - INTERVAL 1 HOUR
GROUP BY status;
```

**Fix:** Scale worker processes, check for blocking queries

---

### Issue: "Migration failed in production"

**DO NOT PANIC. DO NOT RUN AGAIN.**

```bash
# 1. Check migration status
npx prisma migrate status --schema=packages/ar/prisma/schema.prisma

# 2. If partially applied, mark as rolled back
npx prisma migrate resolve --rolled-back <migration_name> \
  --schema=packages/ar/prisma/schema.prisma

# 3. Restore from pre-migration backup if needed
./restore-billing-db.sh /backups/billing/pre-migration.sql.gz

# 4. Debug migration locally
# 5. Fix and redeploy
```

## Change Management

### Schema Changes

**Process:**
1. Create migration in dev: `prisma migrate dev --schema=...`
2. Test in staging
3. Document rollback plan
4. Schedule maintenance window (if needed)
5. Take backup
6. Deploy: `prisma migrate deploy --schema=...`
7. Verify
8. Monitor for 24 hours

**Always include `--schema=packages/ar/prisma/schema.prisma`** ← Pin this everywhere!

---

**Questions? Check:**
- SEPARATE-DATABASE-SETUP.md (initial setup)
- ARCHITECTURE-CHANGE.md (why separate DB)
- SANDBOX-TEST-CHECKLIST.md (testing guide)
