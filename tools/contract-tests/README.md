# contract-tests

Contract validation test crate for event schemas and OpenAPI artifacts.
Contains test suites that verify compatibility of published contracts.

Run:
```bash
cargo test -p contract-tests
cargo test -p contract-tests --test openapi_tests
```

No standalone service; this crate is test-focused.
