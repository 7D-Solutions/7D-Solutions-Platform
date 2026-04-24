# Feature-Flags Contract

> **Owner:** Platform Orchestrator  
> **Status:** v1 — active  
> **Last updated:** 2026-04-24 ([bd-k3318])

## Endpoint

```
GET /api/features?tenant_id={uuid}
Authorization: Bearer <JWT>
```

Returns the full feature-flag map for a tenant, along with the schema version the payload conforms to.

## Response shape (v1)

```json
{
  "tenant_id": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
  "schema_version": 1,
  "flags": {
    "flag_name": true,
    "another_flag": false
  }
}
```

| Field | Type | Notes |
|---|---|---|
| `tenant_id` | UUID string | Echoed from the request parameter |
| `schema_version` | integer | Always `1` for v1 payloads |
| `flags` | `{ [name: string]: boolean }` | Full resolved flag map (per-tenant overrides merged with global defaults) |

## JSON Schema

The machine-readable schema is published at:

```
GET /api/schemas/features/v{N}
```

No authentication required. Returns 404 for unknown versions.

Example: `GET /api/schemas/features/v1`

## Lifecycle rules

1. **`schema_version` is the contract signal.** Frontends must read this field before consuming `flags`. If the version is unknown, fail closed — do not guess at default values.

2. **Additive changes within a version are allowed.** New flag names may appear in `flags` at any time without a version bump. Frontends must not error on unrecognized flag names.

3. **Shape changes require a version bump.** Adding, removing, or changing the type of a top-level field (e.g., `tenant_id`, `schema_version`, `flags`) increments `schema_version` and publishes a new schema at `/api/schemas/features/v{N+1}`. The old version remains available.

4. **Version deprecation.** A version is deprecated with at least 30 days notice via the platform changelog before the schema endpoint returns 410 Gone. Frontends observing a deprecation warning header should migrate before the deadline.

## Error responses

| Status | Meaning |
|---|---|
| 400 | Missing or malformed `tenant_id` parameter |
| 401 | No valid JWT provided |
| 403 | JWT tenant does not match requested `tenant_id` |
| 500 | Internal error — retry with backoff |
