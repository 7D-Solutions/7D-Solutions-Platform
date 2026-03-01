# compliance-export

CLI for exporting tenant audit/ledger data and generating period evidence packs.
Supports export formats such as JSON/CSV for compliance workflows.

Run:
```bash
cargo run -p compliance-export -- --help
cargo run -p compliance-export -- export --tenant t1 --output ./export
```

Config: database connectivity via environment variables consumed by SQLx.
