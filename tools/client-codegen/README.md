# client-codegen

Reads an OpenAPI JSON spec and emits a compilable Rust typed-client crate.

## How it works

1. Each module under `modules/` declares an `openapi_dump` binary
   (`src/bin/openapi_dump.rs`) that prints the module's OpenAPI spec as JSON.
2. Running that binary produces `clients/<module>/openapi.json`.
3. `client-codegen` reads the JSON and writes a full client crate
   (`Cargo.toml`, `src/lib.rs`, typed endpoint files, type files) into the
   output directory.

## Full regen loop

```bash
# 1. Build the codegen tool (once)
./scripts/cargo-slot.sh build -p client-codegen

# 2. For a single module (e.g. reporting):
./scripts/cargo-slot.sh run --bin openapi_dump -p reporting \
  > clients/reporting/openapi.json

./scripts/cargo-slot.sh run --bin client-codegen -- \
  clients/reporting/openapi.json clients/reporting/

# 3. Verify the generated crate compiles:
./scripts/cargo-slot.sh check -p platform-client-reporting
```

Repeat steps 2-3 for each module. The client crate name follows the
pattern `platform-client-<module>` (derived from the OpenAPI `info.title`).

## Modules with openapi_dump binaries

Every module under `modules/` that has a `[[bin]] name = "openapi_dump"`
entry in its `Cargo.toml` can produce a spec. As of this writing that
includes all 26 service modules.

## What gets generated

| File | Contents |
|------|----------|
| `Cargo.toml` | Package metadata, dependencies on `platform-sdk` and `platform-http-contracts` |
| `src/lib.rs` | Module declarations, type re-exports, `PlatformService` trait impls |
| `src/types.rs` (or `types_N.rs`) | Request/response structs from `components.schemas` |
| `src/<tag>.rs` (or `<tag>_N.rs`) | One file per OpenAPI tag with a `<Tag>Client` struct and methods |

Files over 500 lines are automatically split into numbered parts
(`types_1.rs`, `types_2.rs`, etc.).

## Adding a new module to the codegen loop

1. Add `#[utoipa::path]` annotations to all HTTP handlers.
2. Create `src/bin/openapi_dump.rs` with a `#[derive(OpenApi)]` struct
   that lists all annotated paths and schema types.
3. Add a `[[bin]] name = "openapi_dump"` section to the module's `Cargo.toml`.
4. Run the regen loop above.
5. Add the new client crate to the workspace `Cargo.toml` members list.
