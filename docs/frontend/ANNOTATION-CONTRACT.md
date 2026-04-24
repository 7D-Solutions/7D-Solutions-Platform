# Annotation Payload Contract

## Overview

Annotation payloads travel between the PDF-Creation frontend and the `pdf-editor` Rust module. Every payload must carry a `schemaVersion` field so consumers can detect incompatible payloads before attempting deserialization.

## Current Version

**schema_version = 1**

## Wire Shape

The `schemaVersion` field is a positive integer at the top level of each `Annotation` object. When absent, the module treats it as version 1 (backward compatibility for payloads produced before this contract was established).

```json
{
  "schemaVersion": 1,
  "id": "ann-001",
  "x": 120.5,
  "y": 340.0,
  "pageNumber": 1,
  "type": "TEXT",
  "text": "Review this",
  "fontSize": 12
}
```

The full JSON-schema is published at:

```
GET /api/schemas/annotations/v{N}
```

Example: `GET /api/schemas/annotations/v1`

The response carries `Cache-Control: public, max-age=86400` (24 h).

## Version Lifecycle

| Event | Version bump |
|-------|-------------|
| New optional field added | None (no bump needed — existing consumers ignore unknown fields) |
| Required field added or field semantics changed | **MINOR** bump (e.g. 1 → 2) |
| Field removed or type changed incompatibly | **MAJOR** bump — coordinate cross-repo release |

## Rejection Behavior

The `pdf-editor` module rejects any annotation payload where `schemaVersion` falls outside the supported range with HTTP 400:

```
unsupported annotation schema_version=3, this module supports 1-1
```

Frontends should check this error and surface a clear message prompting the user to refresh the page (which will fetch the updated frontend bundle supporting the new schema).

## Cross-Repo Coordination

Schema version bumps require coordinated releases:
1. `pdf-editor` (this repo) adds support for the new version while continuing to accept the old one.
2. PDF-Creation frontend (separate repo) ships `schemaVersion` set to the new value.
3. After the frontend has been fully deployed, the `pdf-editor` minimum supported version can be raised in a subsequent release.

Never drop support for a version until all frontend deployments have migrated.
