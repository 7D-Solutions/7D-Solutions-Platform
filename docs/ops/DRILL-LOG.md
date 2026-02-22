# Restore Drill Log

Chronological record of all restore drills. See `docs/RESTORE-DRILL.md` for procedure.

---

## 2026-02-22 — Restore Drill (bd-12fv, P46-220)

- **Operator:** MaroonHarbor
- **Bead:** bd-12fv (P46-220)
- **Environment:** Local Docker Compose stack (all 18 production-equivalent containers)
- **Backup source:** `2026-02-22_14-34-04` (taken immediately before this drill)
- **Backup size:** 3.7M (18 databases + globals)
- **Restore total elapsed:** 105s (1m 45s)
- **Result:** PASS — 18/18 databases, health audit 18/18 PASS
- **Smoke suite:** PASS — 13/13 checks (dry-run mode; no VPS in local drill)
- **Isolation check:** PASS — 12/12 denial assertions (dry-run mode)

### Backup Step

```bash
BACKUP_DIR=/tmp/7d-backup-drill \
  bash scripts/production/backup_all_dbs.sh
# Output: 18 dump files + globals, 3.7M total
```

All 18 databases dumped successfully, manifest written with SHA-256 checksums.

### Off-Host Shipping

In production, invoke after backup:
```bash
BACKUP_SHIP_METHOD=s3 BACKUP_S3_BUCKET=<bucket> \
  bash scripts/production/backup_ship.sh
```
Not executed in this local drill (no remote target configured). On production VPS,
`install_backup_timer.sh` runs backup + ship on an hourly systemd timer.

### Restore Step

```bash
BACKUP_DIR=/tmp/7d-backup-drill \
  bash scripts/production/restore_drill.sh
```

Manifest checksums verified before restore began. Ephemeral container `7d-drill-postgres`
(Postgres 16) started on isolated `7d-drill-net` network. Databases restored in
DR sequence (platform → financial → modules).

### Per-Database Restore Timings

| DB | Compressed Size | Restore Time | Tier |
|----|----------------|-------------|------|
| auth | 8K | 5s | Platform |
| tenant_registry | 12K | 8s | Platform |
| audit | 208K | 24s | Platform |
| gl | 2.1M | 4s | Critical |
| ar | 576K | 4s | Critical |
| ap | 48K | 1s | Critical |
| payments | 32K | 1s | Critical |
| treasury | 4K | 1s | Critical |
| subscriptions | 24K | 0s | High |
| inventory | 704K | 1s | High |
| fixed_assets | 8K | 1s | High |
| consolidation | 4K | 1s | High |
| notifications | 44K | 0s | Standard |
| projections | 4K | 1s | Standard |
| timekeeping | 12K | 1s | Standard |
| party | 8K | 1s | Standard |
| integrations | 4K | 0s | Standard |
| ttp | 4K | 1s | Standard |
| **TOTAL** | **3.7M** | **105s** | |

### RPO / RTO Assessment

| Tier | RTO Target | Actual | Status |
|------|-----------|--------|--------|
| Platform (auth, tenant_registry, audit) | 2 hours | 37s cumulative | ✅ Well within |
| Critical (GL, AR, AP, Payments, Treasury) | 4 hours | 11s cumulative | ✅ Well within |
| Standard (all others) | 8 hours | 57s remaining | ✅ Well within |
| **Full restore (all 18 DBs)** | 4 hours (critical SLA) | **1m 45s** | ✅ **99% headroom** |

**Note:** These timings reflect current data volume. At 10× data scale, expect
10–20 minutes total — still well within RTO targets. Recheck after each order-of-magnitude growth.

**Audit DB observation:** `audit_db` (208K compressed) took 24s — disproportionate
to size. Cause: high row-count audit log with many sequences to reset. Not a concern
at this volume; recheck at 100× growth.

### Health Audit (post-restore)

```
  ✓  auth                  connected, 8 table(s)
  ✓  tenant_registry       connected, 7 table(s)
  ✓  audit                 connected, 1 table(s)
  ✓  gl                    connected, 23 table(s)
  ✓  ar                    connected, 49 table(s)
  ✓  ap                    connected, 17 table(s)
  ✓  payments              connected, 7 table(s)
  ✓  treasury              connected, 8 table(s)
  ✓  subscriptions         connected, 7 table(s)
  ✓  inventory             connected, 25 table(s)
  ✓  fixed_assets          connected, 10 table(s)
  ✓  consolidation         connected, 8 table(s)
  ✓  notifications         connected, 4 table(s)
  ✓  projections           connected, 3 table(s)
  ✓  timekeeping           connected, 15 table(s)
  ✓  party                 connected, 10 table(s)
  ✓  integrations          connected, 8 table(s)
  ✓  ttp                   connected, 9 table(s)

Health audit results: 18 passed, 0 failed, 0 skipped
```

### Smoke Suite (dry-run)

```bash
bash scripts/production/smoke.sh --host <prod-host> --jwt <staff-jwt>
# Drill: bash scripts/production/smoke.sh --host localhost --dry-run
```
Results: 13/13 checks PASS (liveness, readiness, frontend, data endpoints).

### Isolation Check (dry-run)

```bash
PROD_HOST=<prod-host> bash scripts/production/isolation_check.sh
# Drill: DRY_RUN=true PROD_HOST=localhost bash scripts/production/isolation_check.sh --dry-run
```
Results: 12/12 cross-tenant denial assertions PASS (simulated in dry-run).

### Notes

- Container image pull (postgres:16) added ~60s overhead on first run; subsequent runs
  skip the pull. Factor this into drill estimates on fresh VPS provisioning.
- Off-host shipping not validated in this drill (no S3/SCP target in local env).
  Verify `backup_ship.sh` on production using `install_backup_timer.sh` output.
