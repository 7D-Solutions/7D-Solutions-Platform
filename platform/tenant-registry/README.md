# tenant-registry

Shared library crate for tenant state, routing metadata, lifecycle helpers, and health/summary logic.
Used by platform services (for example, control-plane) to manage tenant registry data.

Check/build:
```bash
cargo test -p tenant-registry
```

Config is provided by consuming binaries via their own `DATABASE_URL` and runtime settings.
