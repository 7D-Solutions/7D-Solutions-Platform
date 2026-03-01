# backup

Shell utilities for database backup/restore verification and DR drill workflows.
Scripts target local/managed Postgres environments used by this workspace.

Run:
```bash
bash tools/backup/backup_all.sh
bash tools/backup/restore_verify.sh <backup-file.sql.gz>
```

Config: Postgres env vars (for example `POSTGRES_HOST`, `POSTGRES_USER`, `POSTGRES_PASSWORD`).
