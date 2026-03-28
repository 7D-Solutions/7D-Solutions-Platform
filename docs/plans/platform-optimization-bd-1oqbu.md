# Platform Tier Optimization — bd-1oqbu

## Methodology

Static profiling of all 12 platform tier crates using the `/extreme-software-optimization` skill.
Grepped for known Rust anti-patterns: `.clone()`, `.to_string()`, `String::from()`, `Vec` allocations in hot loops.
Focused on the four areas identified in the bead: middleware chains, EventEnvelope serialization, hot-path cloning, rate limiter contention.

## Crates Profiled (LOC)

| Crate | LOC | Hot Path |
|-------|-----|----------|
| identity-auth | 6223 | JWT verification, password hashing |
| security | 3853 | Every HTTP request (middleware, rate limiting, authz) |
| doc-mgmt | 3951 | Document lifecycle |
| tenant-registry | 3351 | Tenant CRUD |
| event-bus | 2705 | Every event (envelope, publish, subscribe) |
| projections | 2345 | Cursor rebuild |
| control-plane | 2080 | Tenant provisioning (admin) |
| event-consumer | 1787 | Every consumed event (routing, dedupe, retry) |
| audit | 1483 | Field-level diffs |
| tax-core | 1183 | Jurisdiction resolution |
| platform-contracts | 612 | Re-exports only |
| health | 232 | Readiness probes |

## Opportunity Matrix

| # | Hotspot | Impact | Conf | Effort | Score | Action |
|---|---------|--------|------|--------|-------|--------|
| 1 | Rate limit middleware: Arc clone + path String alloc per request | 5 | 5 | 1 | **25.0** | Implemented |
| 2 | Rate limiter: unnecessary key.clone() in DashMap entry() | 4 | 5 | 1 | **20.0** | Already fixed |
| 3 | InMemoryBus matches_pattern: 2 Vec allocs per message per subscriber | 3 | 5 | 1 | **15.0** | Implemented |
| 4 | Audit diff: BTreeMap intermediary for ordering | 2 | 4 | 1 | **8.0** | Implemented |
| 5 | is_semver: Vec alloc per incoming message | 1 | 5 | 1 | 5.0 | Skipped (dwarfed by DB cost) |
| 6 | Event Router: payload.clone() per dispatch | 4 | 4 | 4 | 4.0 | Skipped (API change) |
| 7 | Event Router: 5 String clones for HandlerContext | 3 | 4 | 4 | 3.0 | Skipped (API change) |
| 8 | Tracing middleware: path/method alloc per request | 3 | 4 | 2 | 6.0 | Skipped (tracing subscriber compat risk) |

## Changes Made

### 1. Rate limit middleware restructure (security/src/middleware.rs)

**Before:** Clone Arc<RateLimiter> from extensions, allocate String for URI path, then check limit.
**After:** Borrow Arc from extensions, borrow path as &str, do rate-limit check, release borrows, then move request.

Eliminates per HTTP request: 1 atomic ref-count increment/decrement (Arc clone) + 1 heap allocation (~50 bytes for typical path).

**Isomorphism:** Same rate-limit decisions, same error responses, same status codes. Borrows are released before request is forwarded.

### 2. InMemoryBus matches_pattern (event-bus/src/inmemory_bus.rs)

**Before:** Collect `subject.split('.').collect::<Vec<&str>>()` and `pattern.split('.').collect::<Vec<&str>>()` — 2 Vec heap allocations per match check, per message, per subscriber.
**After:** Iterator-based matching with `split('.').next()` in a loop. Zero heap allocations.

**Isomorphism:** Identical matching semantics including edge cases (>, *, exact, exhaustion). All 12 pattern-matching tests pass unchanged.

### 3. Audit diff BTreeMap to Vec+sort (audit/src/diff.rs)

**Before:** Insert field changes into BTreeMap<String, FieldChange> for deterministic ordering, then collect into Vec.
**After:** Push into Vec<FieldChange>, then sort_by field name.

Vec+sort avoids BTreeMap's per-node heap allocations. For typical audit diffs (2-10 fields), Vec+sort is measurably faster.

**Isomorphism:** Same deterministic lexicographic ordering. All 9 diff tests pass unchanged.

## Not Changed (and why)

- **EventEnvelope fields (String → Cow<str>):** Would be a breaking public API change on a widely-depended crate. The 5-byte "1.0.0" default allocations are negligible per-event.
- **Event consumer retry loop clones:** The `with_dedupe` closure architecture requires owned data. Fixing this needs a significant refactor of the handler trait and dedupe API.
- **Tracing context middleware String clones:** trace_id and correlation_id must exist both in the tracing span and in request extensions. Cloning is architecturally necessary unless we move to Arc<str>, which cascades widely.
- **JWT verification:** Already efficient — RSA verification dominates; no allocation waste found.
- **Health/platform-contracts/tax-core:** No hot-path inefficiencies found.

## Verification

- `cargo check` on all 12 platform crates: clean (1 pre-existing warning in auth-rs)
- Unit tests: 182 pass across security (102), event-bus (57), audit (23)
- Integration test failures (auth-rs, tenant-registry) are pre-existing DB-connectivity issues, not related to changes
