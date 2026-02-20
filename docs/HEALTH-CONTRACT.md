# Health Endpoint Contract

Every HTTP service in the 7D Solutions Platform MUST expose two health endpoints.

## `/healthz` — Liveness Probe

**Purpose:** Confirm the process is running. No dependency checks.

| Field | Value |
|-------|-------|
| Method | GET |
| Status | Always 200 |
| Body | `{"status":"alive"}` |

Use for: Kubernetes liveness probe, load balancer health, quick process-up checks.

## `/api/ready` — Readiness Probe

**Purpose:** Confirm the service can accept traffic. Checks all critical dependencies.

| Field | Value |
|-------|-------|
| Method | GET |
| Status | 200 (ready/degraded), 503 (down) |

### Response Shape

```json
{
  "service_name": "ar",
  "version": "0.1.0",
  "status": "ready",
  "degraded": false,
  "checks": [
    {
      "name": "database",
      "status": "up",
      "latency_ms": 3
    },
    {
      "name": "nats",
      "status": "up",
      "latency_ms": 1
    }
  ],
  "timestamp": "2026-02-20T12:00:00Z"
}
```

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `service_name` | string | Module/service identifier |
| `version` | string | Cargo package version (`env!("CARGO_PKG_VERSION")`) |
| `status` | enum | `ready` / `degraded` / `down` |
| `degraded` | bool | True only when status is `degraded` |
| `checks` | array | Per-dependency check results |
| `checks[].name` | string | Dependency name (`database`, `nats`, etc.) |
| `checks[].status` | enum | `up` / `down` |
| `checks[].latency_ms` | u64 | Time taken for the check in milliseconds |
| `checks[].error` | string? | Present only when status is `down` |
| `timestamp` | string | ISO 8601 timestamp of the check |

### Status Rules

- All checks `up` → `status: "ready"`, HTTP 200
- Any check `down` → `status: "down"`, HTTP 503

### Required Checks by Service Type

| Service Type | Required Checks |
|-------------|-----------------|
| Module (DB only) | `database` |
| Module (DB + NATS) | `database`, `nats` |
| identity-auth | `database`, `nats` |

## Shared Crate

Use `platform/health` (workspace member) for response types and helpers:

```rust
use health::{healthz, build_ready_response, ready_response_to_axum, db_check};
```

## Verification

```bash
./scripts/verify_health_endpoints.sh
```

Curls every `/healthz` and `/api/ready` endpoint in the running compose stack and asserts the canonical JSON shape.
