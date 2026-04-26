# Golden Artifacts — Provenance

## What these files are

Each `*.golden.png` in this directory is a rasterized PNG of a PDF page that
contains exactly one rendered annotation. They serve as pixel-level baselines:
the `visual_drift` test suite diffs every test run's output against these
goldens and fails if the change ratio exceeds the per-class tolerance.

## How to generate or update

Golden generation requires pdfium to be available (i.e. `PDFIUM_LIB_PATH`
must point to a compatible `libpdfium` shared library — present in the CI
deployment container).

```bash
# Regenerate all goldens
UPDATE_GOLDENS=1 ./scripts/cargo-slot.sh test -p contract-tests -- visual_drift

# Regenerate a single class
UPDATE_GOLDENS=1 ./scripts/cargo-slot.sh test -p contract-tests -- visual_drift_text
```

After regenerating, **review every changed PNG before committing**:

```bash
git diff --stat tools/contract-tests/goldens/
# Open each *.golden.png and verify the rendered annotation looks correct.
git add tools/contract-tests/goldens/
git commit -m "[bd-xxx] Update visual drift goldens: <reason>"
```

Never commit goldens without reviewing them — an unreviewed golden locks in
whatever the renderer happened to produce, including bugs.

## Tolerance by annotation class

| Class                                  | Tolerance |
|----------------------------------------|----------:|
| FREEHAND, SIGNATURE                    | 2.0 %     |
| BUBBLE, CALLOUT, ARROW, SHAPE,         |           |
|   HIGHLIGHT, WHITEOUT                  | 1.0 %     |
| TEXT, STAMP                            | 0.5 %     |

Higher tolerances for FREEHAND/SIGNATURE reflect legitimate rasterization
variability in free-form paths. TEXT/STAMP are strict because font rendering
should be deterministic within a single pdfium build.

## Transient artifacts

On test failure the harness writes:
- `*.actual.png` — what the renderer produced this run
- `*.diff.png`   — red-highlighted diff between actual and golden

These files are listed in `.gitignore` and must not be committed.

## Source inputs

- **Base PDF**: `modules/pdf-editor/tests/fixtures/test.pdf` (US-Letter, 1 page)
- **Fixtures**: `tools/contract-tests/fixtures/pdf-editor-consumer/*.json`
- **Renderer**: `pdf_editor::domain::annotations::render::render_annotations`
- **Rasterizer**: pdfium-render 0.8, 72 DPI (1 pt = 1 px → 612×792 output)
