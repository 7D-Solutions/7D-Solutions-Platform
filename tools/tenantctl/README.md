# tenantctl

CLI for tenant lifecycle and fleet operations (create, activate, verify, migrate, retention).
Works with tenant-registry and related platform services.

Run:
```bash
cargo run -p tenantctl -- --help
cargo run -p tenantctl -- tenant show --tenant t1
cargo run -p tenantctl -- fleet migrate --tenants 10 --parallel 4
```

Config: `TENANTCTL_ROLE`, `TENANTCTL_ACTOR`, and DB/service endpoints.
