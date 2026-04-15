# Security

## CORS policy

Every module that starts through `platform-sdk::ModuleBuilder` must declare
its CORS policy in its `module.toml`. The policy is enforced at startup:
a module with no valid policy crashes with `StartupError::Config(...)` rather
than silently allowing wildcard cross-origin access.

### Policy resolution order

1. **Manifest `[cors]` section** — authoritative. Two forms:
   - `origins = ["https://app.example.com"]` — explicit allowlist.
     An empty list (`origins = []`) is the correct posture for
     server-to-server (non-browser-facing) modules.
   - `origin_pattern = "^https://.*\\.example\\.com$"` — regex predicate.
2. **`CORS_ORIGINS` env var** — operator override, consulted only when the
   manifest has no `[cors]` section. Comma-separated list of exact origins.
   Provided for backwards-compatibility and container-level overrides.

### Wildcard is always an error

Wildcard origins are rejected at startup regardless of environment. This means:

- `origins = ["*"]` in the manifest → `StartupError::Config`
- `origin_pattern = ".*"` (or any pattern matching all origins) → `StartupError::Config`
- `CORS_ORIGINS=*` env var → `StartupError::Config`
- No `[cors]` section and `CORS_ORIGINS` unset → `StartupError::Config`

There is no `ENV=development` bypass. Wildcards are never permitted.

### Adding a new module

**Server-to-server (internal) module** — called only by other backend services,
never directly by a browser:

```toml
[cors]
origins = []
```

The CORS layer rejects all browser-origin requests. All 25 internal platform
modules use this posture.

**Browser-facing module with a known static origin** — called directly by a
browser SPA and the deployed origin is fixed (same for all environments):

```toml
[cors]
origins = ["https://your-app.example.com"]
```

Or use `origin_pattern` for subdomain wildcards:

```toml
[cors]
origin_pattern = "^https://.*\\.your-domain\\.com$"
```

**Browser-facing module with operator-supplied origins** — called directly by
a browser SPA but the deployed origin differs per operator/environment (e.g.
each vertical deploys to their own domain). Do **not** add a `[cors]` section
to `module.toml`. Instead, require operators to set `CORS_ORIGINS` at deploy
time as a comma-separated list of exact origins:

```
CORS_ORIGINS=https://portal.acmewaste.com
```

For local development, set `CORS_ORIGINS=http://localhost:5173` in `.env`.
The module crashes at startup if `CORS_ORIGINS` is unset — this is intentional
(fail-closed). The two platform modules in this category are `customer-portal`
and `pdf-editor`.

## Password hygiene

Production deploy yamls (`docker-compose.data.yml`, `docker-compose.infrastructure.yml`,
`docker-compose.modules.yml`, `docker-compose.platform.yml`, `docker-compose.services.yml`,
`docker-compose.production.yml`, `docker-compose.production-data.yml`) must never carry a
literal password fallback.

**Required form:** `${SERVICE_POSTGRES_PASSWORD}` — no `:-default` suffix.

If a `*_POSTGRES_PASSWORD` environment variable is unset at deploy time, Docker Compose
interpolation fails loudly and the service does not start. This is intentional: a
fail-to-start deploy is noisy and recoverable; a service silently running with a
CI-grade default password is a breach-in-waiting.

**Enforcement:** The `lint-compose-passwords` CI job runs `grep` against all
production-facing compose files on every push and pull request. Any match on
`:-[a-z_]+_pass}` or `:-postgres}` fails the build.

**Secrets at deploy time:** All `*_POSTGRES_PASSWORD` variables are required environment
variables that must be injected from a secrets manager or deployment environment at
deploy time. See the operational runbook (`docs/operations/runbook.md`) for the secrets
rotation procedure.
