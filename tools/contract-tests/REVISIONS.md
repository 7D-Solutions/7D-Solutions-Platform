# contract-tests Revisions

## v1.1.0 — 2026-04-24

Add pdf-editor consumer contract test suite (br-sh7 / bd-af29l integration point).

- New test: `pdf_editor_consumer_test` — loads every JSON under
  `tools/contract-tests/fixtures/pdf-editor-consumer/` and deserializes into the
  production `pdf_editor::domain::annotations::types::Annotation` type.
  A platform-side change that removes or renames a field the frontend sends will fail
  this test and block merge.
- New fixtures: 9 annotation types (ARROW, BUBBLE, CALLOUT, FREEHAND, HIGHLIGHT,
  SHAPE, SIGNATURE, STAMP, TEXT). Produced by PDF-Creation's `extract.mjs` transform.
  WHITEOUT excluded pending `AnnotationType::Whiteout` variant + renderer arm (TODO
  in test).
- Integration point: `bd-af29l` (shared golden-fixture corpus). PDF-Creation's
  `.github/workflows/contract-fixtures-drift.yml` fails if fixtures drift from the
  `toRustAnnotations` transform; a manual PR to this repo is required when that check
  fires.

## v1.0.0 — 2026-03-28

Initial proven release. Event schema and OpenAPI contract validation for the platform.

- 28 passing tests: 2 unit (lib), 14 event schema, 12 OpenAPI spec
- Validates event JSON schemas against contracts/events/ examples
- Validates OpenAPI specs for all platform services (payments, AR, auth, TTP, control-plane, tenant-registry, notifications, subscriptions, inventory, party, integrations-hub, pdf-editor)
- Proof script: `scripts/proof_contract_tests.sh`
