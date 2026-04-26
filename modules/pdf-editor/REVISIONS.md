# pdf-editor — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.


## 2.8.0
- feat(bd-kager): `leader_geometry()` helper in `renderers.rs` computes leader-line endpoints deterministically — origin is always the geometric center of the bubble regardless of shape (oval/circle/square), target is the screen-space anchor flipped to PDF coords. Replaces ad-hoc origin math previously inlined per shape branch.

## 2.7.0
- feat(bd-w0pbp): BUBBLE annotation now supports configurable shape — `oval` (default, prior behaviour), `circle`, `square`. Wire field `bubbleShape` on the annotation payload. Renderer dispatches on shape; consumer contract fixture and JSON Schema updated. Frontend types now include the discriminator.

## 2.6.1
- fix(bd-iah29.1): add `PDFIUM_ABI_CANARY` env knob to `assert_pdfium_abi()` — three modes: `panic` (default, prod), `warn` (log error and continue), `skip` (no-op). Set `PDFIUM_ABI_CANARY=warn` in the dev container to restore previous behaviour where pdf-editor starts despite an unloadable libpdfium.so; prod stays on `panic`.

## 2.6.0
- feat(bd-iah29): startup ABI canary `assert_pdfium_abi()` — verifies pdfium-render crate ↔ packaged libpdfium.so compatibility by binding the library and calling FPDF_LoadMemDocument64 once at boot. Crashes early with an actionable message if the version on disk is incompatible, instead of failing on the first annotation request. No-op when PDFIUM_LIB_PATH is unset.

## 2.5.0
- feat: schema_version field on annotation payload; JSON Schema published at GET /api/schemas/annotations/v{N} ([bd-rxvbr])

## 2.4.0
- feat: add Whiteout annotation type + renderer (bd-rsp36) — new `AnnotationType::Whiteout` variant (SCREAMING_SNAKE_CASE wire: WHITEOUT); renders as solid opaque white rectangle over annotation bounds; exhaustive match arm added in render.rs; consumer contract fixture WHITEOUT.json added and skip removed

## 2.3.5
- chore: rename crate from pdf-editor-rs to pdf-editor (no behavioral change)

## 2.3.4
- fix: signature DRAW coords — multiply signaturePath points by annotation width/height before rendering; fixes sub-1pt blob ([bd-884lm])
- feat: add optional `schemaVersion` field to annotation payload (inbound-only, defaults to "1.0", wire name `schemaVersion`) ([bd-bt4yr])

## 2.3.3
- fix: revert invalid `dynamic` feature flag (doesn't exist in pdfium-render 0.8.37); pdfium dynamic loading is the default behavior without the `static` feature; musl-static binaries cannot dlopen — pdfium render path works on glibc containers (production) but not musl-static dev containers ([bd-didjp])

## 2.3.2
- fix: add `dynamic` feature to pdfium-render — enables `bind_to_library()` / `Pdfium::default()` dlopen path; without this feature the binary panics with "Dynamic loading not supported" even when libpdfium.so is present ([bd-ro834])

## 2.3.1
- chore: trigger cross-watcher rebuild to pick up libpdfium.so mount + PDFIUM_LIB_PATH wiring ([bd-sek3l])

## 2.3.0
- fix: add Linux aarch64 libpdfium.so (musl, from bblanchon/pdfium-binaries chromium/7802); mount into container at /app/libpdfium.so with PDFIUM_LIB_PATH env var; fixes LoadLibraryError panic in Linux containers ([bd-sek3l])

## 2.2.1
- chore: rustfmt reflow + regenerate typed clients (no behavior change)

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| 2.1.5 | 2026-04-01 | Import extract_tenant from platform-sdk instead of local copy (bd-o1a03) |
| Version | Version | Exact SemVer matching the package file |
| Date | Date | ISO date YYYY-MM-DD, not the literal placeholder |
| Bead | Bead | Active bead ID (not bd-xxxx) |
| Summary | What Changed | Concrete — name endpoints, fields, events, behaviors. Not "TODO" or "various improvements." |
| Why | Why | The problem solved or requirement fulfilled. Not "TODO." |
| Proof | (Gate 1) | `scripts/proof_pdf_editor.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.6.1 | 2026-04-25 | bd-iah29.1 | Add `PDFIUM_ABI_CANARY` env knob to `assert_pdfium_abi()`: `panic` (default), `warn` (log + continue), `skip` (no bind). Startup mode logged at info level. | Dev container has arch-mismatched libpdfium.so; canary was crashing the service at startup. `PDFIUM_ABI_CANARY=warn` restores pre-canary partial-failure behaviour without weakening prod. | No |
| 2.2.0 | 2026-04-02 | bd-binuj | Remove dead health.rs (health/ready/version handlers). SDK ModuleBuilder provides these endpoints; the file was unreferenced dead code. | Dead code cleanup — annotation audit revealed health.rs handlers were never mounted after SDK conversion. | No |
| 2.1.4 | 2026-03-31 | bd-5vmu6 | Convert to platform-sdk ModuleBuilder. Replaces manual dotenv/tracing/pool/bus/outbox/middleware/health/shutdown boilerplate with SDK startup sequence. Bus and outbox publisher now configured via module.toml. | SDK batch conversion — eliminate two classes of modules. | No |
| 2.1.3 | 2026-03-31 | bd-vnuvp.9 | Add tenant_id filter to 3 form_fields queries (MAX display_order, SELECT list, SELECT reorder) via form_templates subquery. Defense-in-depth tenant isolation. | P0 tenant isolation sweep: queries must filter by tenant_id to prevent cross-tenant data leakage. | No |
| 2.1.2 | 2026-03-31 | bd-z5rek.3 | Migrate config.rs to ConfigValidator for multi-error startup validation. All config errors reported at once in table format. | Plug-and-play wave 2: consistent startup validation across all modules. | No |
| 2.1.1 | 2026-03-30 | bd-nhmgu | Add openapi_dump utility binary for offline spec generation. | OpenAPI spec validation: offline dump needed for spec extraction and validation. | No |
| 2.1.0 | 2026-03-31 | bd-gvhi3 | OpenAPI spec served at /api/openapi.json with utoipa 5.x annotations on all 15 endpoints. Bearer JWT SecurityScheme documented. ConfigValidator-style startup validation preserved. | Plug-and-play: OpenAPI + startup standardization. | No |
| 2.0.0 | 2026-03-31 | bd-gvhi3 | All error responses migrated to ApiError from platform-http-contracts. list_templates and list_submissions return PaginatedResponse with page/page_size/total_items/total_pages. 50 MB body limit on /api/pdf/render-annotations preserved. Added tenant.rs helper and domain error_conversions.rs. | Plug-and-play: standard response envelopes across platform. | YES: List endpoints now use page/page_size params instead of limit/offset. Error responses now have error, message, request_id fields (was error+message only). |
| 1.0.0 | 2026-03-28 | bd-6hw2k | Initial proof. PDF template management, annotation rendering (text/checkbox/signature overlays), form field extraction, PDF generation from templates with data binding, size validation guard, admin endpoints. 18 unit tests pass, clippy clean. | PDF editor module complete and proven. All gates pass. | No |

## How to read this table

- **Version:** The version in the package file (`Cargo.toml` or `package.json`) after this change.
- **Date:** When the change was committed.
- **Bead:** The bead ID that tracked this work.
- **What Changed:** A concrete description of the change. Name specific endpoints, fields, events, or behaviors affected. Do not write "various improvements" or "minor fixes."
- **Why:** The reason the change was necessary. Reference the problem it solves or the requirement it fulfills.
- **Breaking?:** `No` if existing consumers are unaffected. `YES` if any consumer must change code to handle this version. If YES, include a brief migration note or reference a migration guide.

## Rules

- Add a new row for every version bump. One row per version.
- Do not edit old rows. If a previous change is reversed, add a new row explaining the reversal.
- The commit that bumps the version in the package file must also add the row here. Same commit.
- If the change is breaking (MAJOR version bump), the "Breaking?" column must describe what consumers need to change.