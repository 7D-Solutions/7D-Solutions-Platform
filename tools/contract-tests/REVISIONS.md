# contract-tests Revisions

## v1.0.0 — 2026-03-28

Initial proven release. Event schema and OpenAPI contract validation for the platform.

- 28 passing tests: 2 unit (lib), 14 event schema, 12 OpenAPI spec
- Validates event JSON schemas against contracts/events/ examples
- Validates OpenAPI specs for all platform services (payments, AR, auth, TTP, control-plane, tenant-registry, notifications, subscriptions, inventory, party, integrations-hub, pdf-editor)
- Proof script: `scripts/proof_contract_tests.sh`
