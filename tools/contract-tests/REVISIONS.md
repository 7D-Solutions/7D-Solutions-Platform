# contract-tests Revisions

## v1.1.0 — 2026-04-24

- Golden-fixture corpus for pdf-editor annotations and control-plane capabilities under tools/contract-tests/fixtures/ — consumed by both the platform and PDF-Creation repos ([bd-af29l])
- Coordinate round-trip harness (tests/coord_round_trip.rs) — validates canvas↔PDF point coords survive frontend→wire→Rust→render without unit drift, tolerance-bounded ([bd-87uqu])
- pdf-editor consumer contract test (tests/pdf_editor_consumer_test.rs) — loads fixture annotations and deserializes into production Annotation type ([bd-1rzul])

## v1.0.0 — 2026-03-28

Initial proven release. Event schema and OpenAPI contract validation for the platform.

- 28 passing tests: 2 unit (lib), 14 event schema, 12 OpenAPI spec
- Validates event JSON schemas against contracts/events/ examples
- Validates OpenAPI specs for all platform services (payments, AR, auth, TTP, control-plane, tenant-registry, notifications, subscriptions, inventory, party, integrations-hub, pdf-editor)
- Proof script: `scripts/proof_contract_tests.sh`
