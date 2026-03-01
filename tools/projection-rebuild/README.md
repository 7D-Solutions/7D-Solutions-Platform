# projection-rebuild

CLI tool for rebuilding projection tables from source events with verification helpers.
Supports rebuild, status, verify, and list operations.

Run:
```bash
cargo run -p projection-rebuild -- --help
cargo run -p projection-rebuild -- rebuild tenant_summary
```

Config: Postgres connection env vars used by SQLx and optional RBAC flags.
