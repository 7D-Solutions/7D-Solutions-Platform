# contract-tests

Contract validation test crate for event schemas and OpenAPI artifacts.
Contains test suites that verify compatibility of published contracts.

## Test suites

| Suite | What it verifies |
|---|---|
| `event_schema_tests` | JSON event schemas vs. examples |
| `openapi_tests` | OpenAPI spec validity |
| `consumer_tests` | Consumer contract shape |
| `pdf_editor_consumer_test` | PDF-editor consumer contract vs. platform spec |
| `golden_fixtures` | Annotation + capability fixture deserialization round-trips |
| `coord_round_trip` | Canvas ↔ PDF coordinate transforms, tolerance-bounded (bd-87uqu) |
| `visual_drift` | Per-class pixel-diff of rendered annotation PNGs vs. golden snapshots (bd-dytrn) |

## Running

```bash
./scripts/cargo-slot.sh test -p contract-tests
./scripts/cargo-slot.sh test -p contract-tests --test visual_drift
```

## Visual drift tests

The `visual_drift` suite renders each annotation-class fixture onto a real PDF,
rasterizes page 1 to PNG, and diffs against a committed golden. Tests skip
silently when `PDFIUM_LIB_PATH` is not set (dev without pdfium).

**Tolerances:** FREEHAND/SIGNATURE 2 %, BUBBLE/CALLOUT/ARROW/SHAPE/HIGHLIGHT/WHITEOUT 1 %, TEXT/STAMP 0.5 %.

**Updating goldens** (run in CI where `PDFIUM_LIB_PATH` is set):
```bash
UPDATE_GOLDENS=1 ./scripts/cargo-slot.sh test -p contract-tests -- visual_drift
git diff tools/contract-tests/goldens/   # review every changed PNG
git add tools/contract-tests/goldens/ && git commit
```

See `goldens/PROVENANCE.md` for full details.

No standalone service; this crate is test-focused.
