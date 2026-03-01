# control-plane

HTTP service for tenant lifecycle orchestration and platform billing runs.
Exposes tenant provisioning/summary endpoints and runs tenant-registry migrations on startup.

Run:
```bash
cargo run -p control-plane
```

Config: `DATABASE_URL`, `AR_DATABASE_URL`, `PORT` (default `8092`).
