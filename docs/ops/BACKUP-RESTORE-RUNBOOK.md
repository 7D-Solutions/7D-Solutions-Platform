# Backup and Restore Runbook

**Phase 16: Operational Foundations**

## Purpose

This runbook documents backup and restore procedures for the 7D Solutions Platform. All procedures are designed to support disaster recovery, single-tenant restore, and point-in-time recovery scenarios.

## Table of Contents

1. [Backup Procedures](#backup-procedures)
2. [Restore Procedures](#restore-procedures)
3. [Single-Tenant Restore](#single-tenant-restore)
4. [Point-in-Time Recovery](#point-in-time-recovery)
5. [Verification Steps](#verification-steps)
6. [Troubleshooting](#troubleshooting)

---

## Backup Procedures

### Daily Automated Backup

**Frequency**: Daily at 2:00 AM (configurable via cron)

**Script Location**: `tools/backup/backup_all.sh`

**Backup Artifacts**:
- `ar_YYYY-MM-DD_HH-MM-SS.sql.gz` - AR module database
- `payments_YYYY-MM-DD_HH-MM-SS.sql.gz` - Payments module database
- `subscriptions_YYYY-MM-DD_HH-MM-SS.sql.gz` - Subscriptions module database
- `gl_YYYY-MM-DD_HH-MM-SS.sql.gz` - GL module database
- `notifications_YYYY-MM-DD_HH-MM-SS.sql.gz` - Notifications module database (optional)
- `manifest.json` - Backup metadata
- `backup.log` - Backup execution log

**Retention Policy**: 30 days (configurable, recommend 90 days for financial data)

### Manual Backup

```bash
# Load environment variables
source .env

# Run backup script with custom directory
./tools/backup/backup_all.sh /path/to/backup/dir

# Or use default timestamped directory
./tools/backup/backup_all.sh
```

### Cron Configuration

Add to crontab for automated daily backups:

```cron
# Daily backup at 2:00 AM
0 2 * * * cd /app && source .env && ./tools/backup/backup_all.sh >> /var/log/backup.log 2>&1
```

### Backup Verification

After each backup, verify:

```bash
# Check backup directory
ls -lh backups/YYYY-MM-DD_HH-MM-SS/

# Verify all module backups exist
ls backups/YYYY-MM-DD_HH-MM-SS/*.sql.gz

# Check manifest
cat backups/YYYY-MM-DD_HH-MM-SS/manifest.json

# Verify backup log for errors
grep -i error backups/YYYY-MM-DD_HH-MM-SS/backup.log
```

---

## Restore Procedures

### Full System Restore

**Use Case**: Complete disaster recovery, restore all modules to a backup point.

**Prerequisites**:
- Backup artifacts available
- PostgreSQL instances running
- Database URLs configured in environment

**Steps**:

1. **Stop all application services** (prevent writes during restore):
   ```bash
   docker compose down
   ```

2. **Prepare databases** (drop and recreate):
   ```bash
   # For each module database
   psql -h localhost -U postgres -c "DROP DATABASE IF EXISTS ar_db;"
   psql -h localhost -U postgres -c "CREATE DATABASE ar_db;"

   psql -h localhost -U postgres -c "DROP DATABASE IF EXISTS payments_db;"
   psql -h localhost -U postgres -c "CREATE DATABASE payments_db;"

   psql -h localhost -U postgres -c "DROP DATABASE IF EXISTS subscriptions_db;"
   psql -h localhost -U postgres -c "CREATE DATABASE subscriptions_db;"

   psql -h localhost -U postgres -c "DROP DATABASE IF EXISTS gl_db;"
   psql -h localhost -U postgres -c "CREATE DATABASE gl_db;"

   psql -h localhost -U postgres -c "DROP DATABASE IF EXISTS notifications_db;"
   psql -h localhost -U postgres -c "CREATE DATABASE notifications_db;"
   ```

3. **Restore each module database**:
   ```bash
   BACKUP_DIR="backups/2026-02-15_02-00-00"

   # AR
   gunzip -c "${BACKUP_DIR}/ar_2026-02-15_02-00-00.sql.gz" | \
     psql -h localhost -U postgres -d ar_db

   # Payments
   gunzip -c "${BACKUP_DIR}/payments_2026-02-15_02-00-00.sql.gz" | \
     psql -h localhost -U postgres -d payments_db

   # Subscriptions
   gunzip -c "${BACKUP_DIR}/subscriptions_2026-02-15_02-00-00.sql.gz" | \
     psql -h localhost -U postgres -d subscriptions_db

   # GL
   gunzip -c "${BACKUP_DIR}/gl_2026-02-15_02-00-00.sql.gz" | \
     psql -h localhost -U postgres -d gl_db

   # Notifications
   gunzip -c "${BACKUP_DIR}/notifications_2026-02-15_02-00-00.sql.gz" | \
     psql -h localhost -U postgres -d notifications_db
   ```

4. **Verify restoration**:
   ```bash
   # Check row counts
   psql -h localhost -U postgres -d ar_db -c "SELECT COUNT(*) FROM invoices;"
   psql -h localhost -U postgres -d payments_db -c "SELECT COUNT(*) FROM payment_attempts;"
   psql -h localhost -U postgres -d subscriptions_db -c "SELECT COUNT(*) FROM subscriptions;"
   psql -h localhost -U postgres -d gl_db -c "SELECT COUNT(*) FROM journal_entries;"
   ```

5. **Restart application services**:
   ```bash
   docker compose up -d
   ```

6. **Run smoke tests** (see [Verification Steps](#verification-steps))

---

## Single-Tenant Restore

**Use Case**: Restore data for a single tenant/customer without affecting other tenants.

**Prerequisites**:
- Backup containing the target tenant's data
- Tenant ID known (app_id or tenant_id)
- Understanding of data dependencies (invoices → payments → GL)

**Steps**:

1. **Identify tenant data in backup**:
   ```bash
   TENANT_ID="tenant-123"
   BACKUP_DIR="backups/2026-02-15_02-00-00"

   # Create temporary directory for tenant data
   mkdir -p temp_restore/${TENANT_ID}
   ```

2. **Extract tenant-specific data from each module**:

   **AR Module** (invoices, customers):
   ```bash
   # Extract tenant invoices
   gunzip -c "${BACKUP_DIR}/ar_2026-02-15_02-00-00.sql.gz" | \
     grep -A 50 "COPY.*invoices" | \
     awk -v tid="${TENANT_ID}" '$0 ~ tid' > temp_restore/${TENANT_ID}/ar_invoices.sql

   # Extract tenant customers
   gunzip -c "${BACKUP_DIR}/ar_2026-02-15_02-00-00.sql.gz" | \
     grep -A 50 "COPY.*ar_customers" | \
     awk -v tid="${TENANT_ID}" '$0 ~ tid' > temp_restore/${TENANT_ID}/ar_customers.sql
   ```

   **Payments Module**:
   ```bash
   gunzip -c "${BACKUP_DIR}/payments_2026-02-15_02-00-00.sql.gz" | \
     grep -A 50 "COPY.*payment_attempts" | \
     awk -v tid="${TENANT_ID}" '$0 ~ tid' > temp_restore/${TENANT_ID}/payments.sql
   ```

   **Subscriptions Module**:
   ```bash
   gunzip -c "${BACKUP_DIR}/subscriptions_2026-02-15_02-00-00.sql.gz" | \
     grep -A 50 "COPY.*subscriptions" | \
     awk -v tid="${TENANT_ID}" '$0 ~ tid' > temp_restore/${TENANT_ID}/subscriptions.sql
   ```

   **GL Module**:
   ```bash
   gunzip -c "${BACKUP_DIR}/gl_2026-02-15_02-00-00.sql.gz" | \
     grep -A 50 "COPY.*journal_entries" | \
     awk -v tid="${TENANT_ID}" '$0 ~ tid' > temp_restore/${TENANT_ID}/gl_entries.sql
   ```

3. **Delete current tenant data** (if corrupted):
   ```sql
   -- AR
   DELETE FROM invoices WHERE app_id = 'tenant-123';
   DELETE FROM ar_customers WHERE app_id = 'tenant-123';

   -- Payments
   DELETE FROM payment_attempts WHERE app_id = 'tenant-123';

   -- Subscriptions
   DELETE FROM subscriptions WHERE tenant_id = 'tenant-123';

   -- GL
   DELETE FROM journal_entries WHERE tenant_id = 'tenant-123';
   ```

4. **Restore tenant data**:
   ```bash
   # Restore in dependency order: Subscriptions → AR → Payments → GL
   psql -h localhost -U postgres -d subscriptions_db < temp_restore/${TENANT_ID}/subscriptions.sql
   psql -h localhost -U postgres -d ar_db < temp_restore/${TENANT_ID}/ar_customers.sql
   psql -h localhost -U postgres -d ar_db < temp_restore/${TENANT_ID}/ar_invoices.sql
   psql -h localhost -U postgres -d payments_db < temp_restore/${TENANT_ID}/payments.sql
   psql -h localhost -U postgres -d gl_db < temp_restore/${TENANT_ID}/gl_entries.sql
   ```

5. **Verify tenant data integrity**:
   ```sql
   -- Check invoice count
   SELECT COUNT(*) FROM invoices WHERE app_id = 'tenant-123';

   -- Check payment attempts
   SELECT COUNT(*) FROM payment_attempts WHERE app_id = 'tenant-123';

   -- Verify GL balance
   SELECT
     SUM(debit_cents) - SUM(credit_cents) as balance
   FROM journal_entries
   WHERE tenant_id = 'tenant-123';
   ```

**Note**: Single-tenant restore is complex due to cross-module references. Consider full restore + data deletion as an alternative for critical scenarios.

---

## Point-in-Time Recovery

**Use Case**: Restore system state to a specific point in time (e.g., before a data corruption event).

**Prerequisites**:
- Backup from before the target timestamp
- Transaction logs (if available)
- Knowledge of the corruption event timestamp

**Steps**:

1. **Identify the closest backup** before the target time:
   ```bash
   ls -lt backups/ | head -10
   ```

2. **Perform full system restore** (see [Full System Restore](#full-system-restore))

3. **If transaction logs available**, replay logs up to target timestamp:
   ```bash
   # This requires PostgreSQL WAL archiving to be enabled
   # See PostgreSQL PITR documentation for details
   ```

4. **Verify data state** at target timestamp

**Note**: Point-in-time recovery requires WAL archiving to be configured. See PostgreSQL documentation for setup: https://www.postgresql.org/docs/current/continuous-archiving.html

---

## Verification Steps

After any restore operation, run these verification steps:

### 1. Database Connectivity
```bash
psql -h localhost -U postgres -d ar_db -c "SELECT 1;"
psql -h localhost -U postgres -d payments_db -c "SELECT 1;"
psql -h localhost -U postgres -d subscriptions_db -c "SELECT 1;"
psql -h localhost -U postgres -d gl_db -c "SELECT 1;"
```

### 2. Row Count Validation
```bash
# Compare with manifest or known counts
psql -h localhost -U postgres -d ar_db -c "SELECT 'invoices' as table, COUNT(*) FROM invoices UNION ALL SELECT 'ar_customers', COUNT(*) FROM ar_customers;"

psql -h localhost -U postgres -d payments_db -c "SELECT 'payment_attempts' as table, COUNT(*) FROM payment_attempts;"

psql -h localhost -U postgres -d subscriptions_db -c "SELECT 'subscriptions' as table, COUNT(*) FROM subscriptions;"

psql -h localhost -U postgres -d gl_db -c "SELECT 'journal_entries' as table, COUNT(*) FROM journal_entries;"
```

### 3. Data Integrity Checks
```bash
# Run platform invariants
cargo test -p ar-rs --test invariants
cargo test -p payments-rs --test invariants
cargo test -p subscriptions-rs --test invariants
cargo test -p gl-rs --test invariants
```

### 4. Cross-Module Consistency
```bash
# Verify invoice → payment linkage
psql -h localhost -U postgres << EOF
\c ar_db
SELECT COUNT(*) as finalized_invoices FROM invoices WHERE finalization_status = 'finalized';

\c payments_db
SELECT COUNT(*) as payment_attempts FROM payment_attempts;
EOF
```

### 5. GL Balance Verification
```sql
-- GL must always balance (debits = credits)
SELECT
  SUM(debit_cents) as total_debits,
  SUM(credit_cents) as total_credits,
  SUM(debit_cents) - SUM(credit_cents) as balance
FROM journal_entries;
-- balance MUST be 0
```

### 6. Application Health Check
```bash
curl http://localhost:8086/api/health  # AR
curl http://localhost:8082/api/health  # Payments
curl http://localhost:8084/api/health  # Subscriptions
curl http://localhost:8088/api/health  # GL
```

---

## Troubleshooting

### Backup Script Fails

**Symptom**: `backup_all.sh` exits with error

**Common Causes**:
1. Database URL not set in environment
2. pg_dump not installed
3. Insufficient disk space
4. PostgreSQL connection refused

**Solutions**:
```bash
# Check environment variables
env | grep DATABASE_URL

# Install pg_dump (if missing)
brew install postgresql  # macOS
apt-get install postgresql-client  # Ubuntu

# Check disk space
df -h

# Test database connectivity
psql "${AR_DATABASE_URL}" -c "SELECT 1;"
```

### Restore Fails with Duplicate Key Error

**Symptom**: `psql` restore fails with "duplicate key value violates unique constraint"

**Cause**: Target database is not empty

**Solution**:
```bash
# Drop and recreate database
psql -h localhost -U postgres -c "DROP DATABASE ar_db;"
psql -h localhost -U postgres -c "CREATE DATABASE ar_db;"

# Retry restore
gunzip -c backup.sql.gz | psql -h localhost -U postgres -d ar_db
```

### Single-Tenant Restore Incomplete

**Symptom**: Tenant data partially restored, cross-module references broken

**Cause**: Data extraction incomplete or restoration order incorrect

**Solution**:
- Always restore in dependency order: Subscriptions → AR → Payments → GL
- Verify foreign key constraints after restore
- Consider full restore + data deletion as safer alternative

### Backup Artifacts Corrupted

**Symptom**: `gunzip` fails or `psql` fails to parse SQL

**Cause**: Incomplete backup or file corruption

**Solution**:
```bash
# Verify backup file integrity
gunzip -t backup.sql.gz

# If corrupted, restore from previous day's backup
ls -lt backups/ | head -5
```

---

## Emergency Contacts

- **Platform Lead**: @PearlOwl
- **Database Admin**: (TBD)
- **On-Call Rotation**: (TBD)

---

## Appendix

### Backup Storage Recommendations

- **Local Backups**: `./backups/` (30-day retention)
- **Remote Backups**: AWS S3, Google Cloud Storage, or equivalent (90-day retention for financial data)
- **Encryption**: Use `gpg` for backup encryption in production:
  ```bash
  gzip backup.sql | gpg --encrypt --recipient ops@7dsolutions.com > backup.sql.gz.gpg
  ```

### Monitoring and Alerts

Set up alerts for:
- Backup script failures (check exit code)
- Backup size anomalies (sudden increase/decrease)
- Missing daily backups
- Disk space warnings (backup directory)

### Compliance Requirements

- **SOX**: 7-year retention for financial transaction data (invoices, payments, GL)
- **GDPR**: 3-year retention for customer data, with right to erasure
- **SOC2**: Backup verification and disaster recovery testing (quarterly)

---

## References

- PostgreSQL Backup Documentation: https://www.postgresql.org/docs/current/backup.html
- PostgreSQL PITR: https://www.postgresql.org/docs/current/continuous-archiving.html
- Phase 16 Specification: `docs/phases/PHASE-16.md`
- Mutation Classes: `docs/governance/MUTATION-CLASSES.md`

## Changelog

- **2026-02-16**: Initial runbook (Phase 16)
