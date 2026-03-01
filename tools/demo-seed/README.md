# demo-seed

Deterministic demo-data seeder that writes repeatable tenant data through module APIs.
Same tenant+seed input produces the same resource set and digest.

Run:
```bash
cargo run -p demo-seed -- --tenant t1 --seed 42 --ar-url http://localhost:8086
cargo run -p demo-seed -- --tenant t1 --seed 42 --print-hash
```

Config: `DEMO_TENANT_ID`, `AR_BASE_URL`.
