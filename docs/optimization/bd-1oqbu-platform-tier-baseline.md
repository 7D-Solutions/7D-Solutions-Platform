# Platform Tier Optimization Baseline

**Bead**: bd-1oqbu
**Date**: 2026-03-28
**Crates**: event-bus, event-consumer, security, platform-contracts

## Benchmark Results (Baseline)

### EventEnvelope Operations
| Operation | Time (median) |
|-----------|---------------|
| envelope_new | 1.42 µs |
| envelope_with_all_builders | 2.47 µs |
| envelope_serialize_json | 474 ns |
| envelope_deserialize_json | 725 ns |
| envelope_clone | 241 ns |

## Hot Path Analysis

### 1. Event Dispatch (event-consumer/router.rs)
Every event dispatch creates a `HandlerContext` with these allocations:
- `tenant_id.clone()` - String clone
- `source_module.clone()` - String clone
- `correlation_id.clone()` - Option<String> clone
- `causation_id.clone()` - Option<String> clone
- `schema_version.clone()` - String clone
- `envelope.payload.clone()` - serde_json::Value clone

**Cost**: ~5-6 String clones + Value clone per event

### 2. EventEnvelope Creation
Every envelope allocates:
- `"1.0.0".to_string()` x2 for source_version/schema_version defaults
- Strings for tenant_id, source_module, event_type
- Uuid::new_v4() and Utc::now()

### 3. NatsBus Publish
- `subject.to_string()` required (async_nats ToSubject only impl for &'static str, not &str)

### 4. Security Middleware
- ClaimsMiddleware clones Arc<JwtVerifier> per request (cheap)
- VerifiedClaims cloned into request extensions
- RateLimiter uses DashMap (lock-free, well-optimized)

## Opportunity Matrix

| Hotspot | Impact | Confidence | Effort | Score | Decision |
|---------|--------|------------|--------|-------|----------|
| Builder methods String→impl Into | 2 | 4 | 2 | 4.0 | IMPLEMENTED |
| router.rs HandlerContext clones | 3 | 4 | 4 | 3.0 | DEFER (handler sig change) |
| EventEnvelope field types | 3 | 3 | 5 | 1.8 | DEFER (breaking change) |
| NatsBus subject alloc | 2 | 2 | 5 | 0.8 | SKIP (required by API) |

## Changes Implemented

### 1. Builder Methods Accept `impl Into<String>` (event-bus v1.0.1)

Changed builder methods from `String` to `impl Into<String>`:
- `with_source_version`
- `with_schema_version`
- `with_actor`

**Before:**
```rust
.with_schema_version(version.clone()) // must clone if reusing
.with_schema_version("1.0.0".to_string()) // must call to_string
```

**After:**
```rust
.with_schema_version(version) // move owned String directly
.with_schema_version("1.0.0") // cleaner, converts via into()
```

**Benefit**:
- Ergonomic: callers write cleaner code
- Performance: callers with owned `String` avoid clone when passing to builder

**Isomorphism Proof**:
- Ordering preserved: yes (builder chain unchanged)
- Tie-breaking unchanged: N/A
- Floating-point: N/A
- Golden outputs: Verified via existing test suite (57 unit + 14 integration + 11 doc-tests)

## Verification

```bash
./scripts/cargo-slot.sh test -p event-bus          # All 82 tests pass
./scripts/cargo-slot.sh test -p security           # All tests pass
./scripts/cargo-slot.sh test -p platform_contracts # All 156 tests pass
./scripts/cargo-slot.sh check --workspace          # Compiles clean
```

## Future Work (Deferred)

1. **HandlerContext borrowing**: Change handler signature to accept `&EventEnvelope` instead of cloned context. Would eliminate all String clones in router. Requires breaking change to handler trait.

2. **EventEnvelope Cow fields**: Change String fields to `Cow<'static, str>` for zero-cost static defaults. Requires serde customization and breaking change.

3. **Arc<str> for frequently-cloned fields**: If same envelope is dispatched to multiple handlers, Arc<str> would pay clone cost once. Current architecture dispatches to single handler.
