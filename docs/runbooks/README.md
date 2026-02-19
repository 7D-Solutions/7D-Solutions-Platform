# Runbooks

Operational runbooks for the 7D Solutions Platform.

## Available Runbooks

| Runbook | Description |
|---------|-------------|
| [BACKUP-RESTORE-RUNBOOK.md](../ops/BACKUP-RESTORE-RUNBOOK.md) | Per-tenant DB backup and restore procedures |
| [RELEASE-PROMOTION.md](../ops/RELEASE-PROMOTION.md) | Staging → Production promotion workflow |
| [ALERT-THRESHOLDS.md](../ops/ALERT-THRESHOLDS.md) | Alert threshold configuration and escalation |

## Runbook Structure

Each runbook should include:

1. **Purpose** — what this runbook covers
2. **Prerequisites** — access, tools, credentials needed
3. **Procedure** — step-by-step commands
4. **Verification** — how to confirm success
5. **Rollback** — what to do if the procedure fails

## Adding a Runbook

Create a new `.md` file in this directory following the structure above. Link it in the table above.
