# Runbooks

Operational runbooks for the 7D Solutions Platform.

## Available Runbooks

| Runbook | Description |
|---------|-------------|
| [incident_response.md](incident_response.md) | Incident response: UNKNOWN resolution, DLQ replay, invariant violations; Decision Trees for rollback and webhook failure |
| [support_checklist.md](support_checklist.md) | On-call shift start/end checklist and daily ops tasks |
| [disaster_recovery.md](disaster_recovery.md) | DR runbook with RPO/RTO targets + quarterly drill; Decision Trees for rollback vs DR and post-recovery |
| [backup_restore.md](backup_restore.md) | Scripted backup/restore with smoke verification |
| [projection_rebuild.md](projection_rebuild.md) | Projection rebuild via CLI tool and admin endpoints |
| [BACKUP-RESTORE-RUNBOOK.md](../ops/BACKUP-RESTORE-RUNBOOK.md) | Legacy per-tenant DB backup/restore reference |
| [RELEASE-PROMOTION.md](../ops/RELEASE-PROMOTION.md) | Staging → Production promotion workflow |
| [ALERT-THRESHOLDS.md](../ops/ALERT-THRESHOLDS.md) | Alert threshold configuration and escalation |
| [key_rotation.md](key_rotation.md) | Zero-downtime JWT key and webhook secret rotation with overlap window |
| [billing_verification.md](billing_verification.md) | Production billing/payment verification cycle and idempotency proof (Tilled test mode) |

## Runbook Structure

Each runbook should include:

1. **Purpose** — what this runbook covers
2. **Prerequisites** — access, tools, credentials needed
3. **Procedure** — step-by-step commands
4. **Verification** — how to confirm success
5. **Rollback** — what to do if the procedure fails

## Adding a Runbook

Create a new `.md` file in this directory following the structure above. Link it in the table above.
