# Stream D — 7D Platform QBO Bidirectional Sync

**Owner:** LavenderWaterfall
**Companion:** Huber's plan at `.review-scratch/STREAM-D-HUBER.md` in Huber Power repo
**Status:** HARDENED v2 (post Grok + Codex adversarial review)

---

## 1. Scope boundary

**Platform owns** the sync engine, OAuth lifecycle, authority registry, push-attempt ledger, conflict detection + observation layer, conflict queue API, explicit per-entity push handlers, idempotency, sync-health surface.

**Platform does NOT own** any UI component, entity mapping for a specific vertical, or vertical-specific business rules. REST API + stable schema + docs + sample payloads — verticals render their own.

## 2. Design baseline (locked with James)

- Per-type authority flip. One side is authoritative per tenant × entity type at any given time.
- **Bidirectional** flip — not a one-way ratchet. Registry holds current state + version.
- Origin is metadata, not permission.
- Conflict classes: unexpected creation / edit / deletion.
- Bulk actions: "Accept from QBO" / "Reject, push HP" / "Ignore".
- Disconnect is warn-and-allow (flip-to-HP + OAuth disconnect).
- **No shared React layer.** Each vertical renders its own UI against the platform API.

## 3. Interface decisions

1. **Conflict record storage** → Platform holds full record (both values, metadata, resolution state). Huber queries via REST + receives NATS event for reactivity. REST replay endpoint for subscriber gap recovery. Retention: 90-day hot, auto-archive after that; hard purge at 365 days. Size cap: 256 KB per value column.

2. **Conflict notification delivery** → NATS events under existing outbox. Event names are **unversioned** to match existing code (`qbo.entity.synced` convention, not `.v1`): `integrations.sync.conflict.detected`, `integrations.sync.conflict.resolved`, `integrations.sync.push.failed`, `integrations.sync.authority.changed`. Push-succeeded events dropped as low-signal.

3. **Push failure surfacing** → In-band on sync HTTP push (result taxonomy below). Async retries publish `push.failed` events. Every push regardless of channel is recorded in the ledger.

4. **Bulk resolution semantics** → Best-effort per-item, cap 100 per call. **Server always computes the deterministic key** = `conflict_id + action + authority_version`; caller-supplied `idempotency_key` aliases to (not replaces) the server key. Response has per-item outcome. Failed items retry-safe on either key. Not transactional across items; transactional within each item's DB side (QBO leg is non-rollbackable external).

5. **In-flight flip handling — version-checked with split outcomes.** Authority table carries `authority_version BIGINT`. Every push stamps its accepted `authority_version` in the ledger at accept time. Outcome depends on WHEN the flip occurs:
   - **Flip before outbound QBO call begins** → push returns `AuthoritySuperseded { new_authority_version }`. Ledger status = `superseded`. No external write. Pending outbox row for the entity_type is quiesced with `failure_reason='authority_superseded'`.
   - **Flip after QBO write succeeds** → QBO write is non-rollbackable. Ledger status = `completed_under_stale_authority`. System immediately reconciles the written state against the now-authoritative side using canonical projection / comparable hash. If equivalent on authoritative fields, auto-close with no admin visibility. If divergent, open a conflict row (`class='edit'`, `detected_by_source='push_attempt'`) for admin resolution.
   - Flip acquires a PostgreSQL advisory lock on `(app_id, provider, entity_type)` to serialize flips; version bump is atomic within the locked transaction.

6. **Service-account label** → **Dropped from scope.** Intuit exposes no per-request actor display name. QBO audit log shows "System Administration" for third-party-app activity. Per-vertical branding, if ever needed, is achieved via separate Intuit apps (separate client_ids), not runtime labels. Docs will state this clearly.

## 4. Data model

All new tables, not extensions to existing ones (except OAuth reconnect fix in Phase 0).

### `integrations_sync_authority`
Per (app_id, provider, entity_type): `authority_side` ('external' | 'internal'), `authority_version BIGINT` (monotonic, bumps on every flip), `last_flipped_at`, `last_flipped_by`. Unique (app_id, provider, entity_type).

### `integrations_sync_push_attempts`
Per push attempt: `id`, `app_id`, `provider`, `entity_type`, `entity_id`, `external_id?`, `direction` ('internal_to_external' | 'external_to_internal'), `operation` ('create' | 'update' | 'delete' | 'resolve_accept_external' | 'resolve_reject_push_internal' | 'resolve_ignore'), `authority_version`, `authority_side`, `conflict_id?` (FK later), `request_id`, `caller_idempotency_key?`, `request_fingerprint` (sha256 of normalized intent), `payload JSONB`, `source_channel` ('api' | 'bulk_resolve' | 'retry_worker' | 'reconcile_worker'), `status` ('accepted' | 'inflight' | 'succeeded' | 'failed' | 'superseded' | 'completed_under_stale_authority'), `error_code?`, `error_detail?`, `accepted_at`, `started_at?`, `finished_at?`, **result markers** for detector correlation: `result_sync_token?`, `result_last_updated_time?`, `result_projection_hash?`.

Indexes:
- `UNIQUE (app_id, provider, request_id)`
- **Partial unique** `(app_id, provider, entity_type, entity_id, operation, authority_version, request_fingerprint) WHERE status IN ('accepted','inflight','succeeded')` — prevents duplicate in-flight or successful pushes for same intent.
- `(app_id, provider, status, accepted_at DESC)` — worker scans
- `(conflict_id)`
- `(app_id, provider, entity_type, entity_id, accepted_at DESC)` — timeline/debug

Retention: hot 90 days; hard purge 365.

### `integrations_sync_conflicts`
Per detected drift: `id`, `app_id`, `provider`, `entity_type`, `class` ('creation' | 'edit' | 'deletion'), `external_id`, `internal_id`, `external_value JSONB`, `internal_value JSONB`, `detected_at`, `detected_by_source` ('webhook' | 'cdc' | 'push_attempt' | 'manual'), `status` ('pending' | 'resolved' | 'ignored' | 'unresolvable'), `resolution_action?`, `resolved_at?`, `resolved_by?`.

Indexes: `(app_id, provider, status, detected_at DESC)`. 256 KB cap per value column.

### `integrations_sync_observations`
Cross-channel dedupe + field-level compare substrate. Per observed entity state: `id`, `app_id`, `provider`, `entity_type`, `entity_id`, `realm_id`, `sync_token?`, `last_updated_time?`, `tombstone_flag`, `fingerprint` (sha256 of canonical identity fields), `projection_version` (schema version of the normalized comparable form), `comparable_hash` (sha256 of normalized business-field projection used for intent compare), `source_channel` ('webhook' | 'cdc' | 'full_resync'), `observed_at`, `payload JSONB` (raw, for audit/debug; detector drives logic from `comparable_hash`, not raw payload).

Unique `(app_id, provider, entity_type, entity_id, fingerprint)` — drops duplicate observations regardless of channel.

Fingerprint composition: `sha256(app_id | provider | realm_id | entity_type | entity_id | sync_token | last_updated_time | tombstone_flag)`. **Fallback** when `sync_token` or `last_updated_time` is absent: include a normalized-payload hash component so distinct states don't dedupe collisions into one row.

Retention: 30 days hot, purge after.

### Phase 0 — OAuth reconnect fix (migration changes existing)

`repo::upsert_connection_by_app_provider` using `INSERT … ON CONFLICT (app_id, provider) DO UPDATE SET realm_id, tokens, expiries, scopes, connection_status='connected', updated_at=NOW()`. Preserves row id.

**`(provider, realm_id)` uniqueness becomes partial** — applies only where `connection_status = 'connected'` — so a previously disconnected tenant doesn't lock a QBO realm out from reconnecting on a different tenant. The full `(app_id, provider)` unique stays as-is to block double-connect within one tenant.

Additional hardening in same bead: callback requires `state` parameter validation (currently falls through to `"default"` tenant — pre-existing anti-CSRF hole at `http/oauth.rs:100,106,191` / verified also at `oauth.rs:133`).

No data migration needed for the live sandbox row (`9341456702925820` from 2026-04-14) — upsert reactivates in place.

### `integrations_sync_jobs` (Phase 3)
Narrow operational index for non-push workers (CDC poll, webhook handler, refresh worker, token-refresh worker). Push attempts aggregate from the ledger, but these jobs have no ledger row — they don't push.

Columns: `(app_id, provider, job_name, last_started_at, last_successful_at, last_failed_at, records_processed_last_run, consecutive_failures, last_error_code?, updated_at)`. Unique `(app_id, provider, job_name)`. Each worker tick UPSERTs.

Not an alerting system — Huber's admin page queries it for "sync health" display. Alerting stays in metrics.rs + Prometheus.

### `integrations_outbox` (extend)
Add `failure_reason TEXT` column (enum-like text: 'authority_superseded' | 'needs_reauth' | 'retry_exhausted' | 'bus_publish_failed' | null). Current schema has `error_message` (free text) but no structured reason code. DLQ API (`GET /sync/dlq`) reads `failure_reason` for filtering.

### Dropped from earlier draft

- `service_account_label` column — Intuit has no mechanism to use it (Codex Q5 research on Intuit OAuth + audit log docs).

## 5. API surface

All under `/api/integrations/sync/*`. URL shape unified as `/sync/{resource}`.

- `GET /sync/authority` — list per-tenant authority state + `authority_version`.
- `PUT /sync/authority/{entity_type}` — flip (atomic with advisory lock + version bump); emits `integrations.sync.authority.changed`.
- `POST /sync/push/{entity_type}` — **explicit per-entity handlers** routed by path (`customer` | `invoice` | `payment`). Not trait-dispatched — each handler lives in its own module. Returns push-result taxonomy (below).
- `GET /sync/conflicts` — list with filters (type, class, age, status).
- `POST /sync/conflicts/{id}/resolve` — single resolution.
- `POST /sync/conflicts/bulk-resolve` — items: `[{conflict_id, action, expected_authority_version?, idempotency_key?}]`, cap 100. Explicit match on `(entity_type, action)` in `resolve_service.rs`.
- `GET /sync/push-attempts` — filterable ledger for debugging/audit.
- `GET /sync/dlq` — outbox rows filtered by `failure_reason` ('authority_superseded' | 'needs_reauth' | 'retry_exhausted' | 'bus_publish_failed').
- `GET /sync/jobs` — non-push worker health (Phase 3).

All auth-gated. New capabilities:
- `integrations.sync.authority.flip` — sensitive
- `integrations.sync.conflict.resolve` — moderate
- `integrations.sync.push` — automated (service tokens OK)
- `integrations.sync.read` — read (status, conflict list, ledger)

## 6. Push-result taxonomy

Every push handler returns one of:
- `Success { external_id, sync_token, last_updated_time }`
- `Conflict { conflict_id }` — detector wrote a conflict row; caller reads it. Includes field-level-compare conflicts (Phase 2 stale-retry guard).
- `ValidationError { code, detail }` — QBO rejected shape (bad doc number, bad customer ref).
- `ClosedPeriod { period_end_date }` — QBO books closed at that date.
- `NeedsReauth` — connection_status != 'connected'; surfaced fast without hitting Intuit.
- `RateLimited { retry_after_seconds? }` — 429 from QBO; caller schedules retry.
- `StaleObject { current_sync_token }` — SyncToken mismatch and retry exhausted OR field-level compare on retry detected touched-field drift.
- `DuplicateIntent { existing_request_id }` — ledger's partial unique detected this push was already in-flight or succeeded.
- `AuthoritySuperseded { new_authority_version }` — flip raced this push **before outbound call**; no external write occurred.
- `CompletedUnderStaleAuthority { external_id, reconciliation: 'auto_closed' | 'conflict_raised', conflict_id? }` — flip occurred **after** QBO write succeeded. System auto-reconciled. If auto-closed, no admin action needed. If conflict_raised, `conflict_id` surfaces for resolution.
- `ExternalNotFound` — update/delete target missing in QBO (already removed or never existed).
- `InternalError { code, detail, retryable: bool }` — non-QBO failure (DB, serialization, outbox).
- `UnknownFailure { details }` — unmapped Intuit fault or transport anomaly; clients must NOT infer success from unclassified errors.

Maps to Intuit fault codes via existing `classify_error` / `parse_api_error` in `qbo/mod.rs`, extended with the new variants. Exposed via NATS on async paths as `integrations.sync.push.failed` with the taxonomy code.

Note: ledger status `aborted` (was in v1) is renamed `completed_under_stale_authority` for accurate semantics; all other ledger statuses map 1:1 to a push-result variant.

## 7. CDC/webhook dedupe — deterministic merge rule + detector correlation

### Dedupe rule

- Fingerprint = `sha256(app_id | provider | realm_id | entity_type | entity_id | sync_token | last_updated_time | tombstone_flag)`.
- **Fallback** when `sync_token` or `last_updated_time` missing: include normalized payload hash so distinct states don't collide.
- Webhook path: schedule immediate fetch → normalize → write observation.
- CDC path: each paged entity → normalize → write observation.
- `integrations_sync_observations` unique on fingerprint rejects duplicates regardless of channel.
- On differing fingerprint for the same `(app_id, provider, entity_type, entity_id)`: process in order of `last_updated_time → sync_token → observed_at`.
- Existing two-level webhook dedupe (body hash + CloudEvent id in `qbo_normalizer.rs`) is kept unchanged — this is a cross-channel layer on top.

### Detector-ledger correlation (suppress self-echo)

When the detector evaluates an observation, it queries `integrations_sync_push_attempts` for a matching push using **result markers**, not time-only. The correlation also catches **orphaned writes** — where the QBO call succeeded but the ledger transition to `succeeded` rolled back, leaving the row at `failed` or `unknown_failure`:

```
SELECT status, conflict_id FROM integrations_sync_push_attempts
WHERE app_id = $1 AND provider = $2
  AND entity_type = $3 AND entity_id = $4
  AND status IN ('succeeded', 'completed_under_stale_authority', 'failed', 'unknown_failure')
  AND result_sync_token = $observation.sync_token
  AND result_last_updated_time = $observation.last_updated_time
  AND result_projection_hash = $observation.comparable_hash
  AND finished_at > NOW() - INTERVAL '1 hour'
```

- Time window (1 hour) bounds query cost — does not decide correctness.
- Marker equality on SyncToken + LastUpdatedTime + projection hash proves the observation is our own push reflecting back.
- If match with `status IN ('succeeded', 'completed_under_stale_authority')` → self-echo, suppress.
- If match with `status IN ('failed', 'unknown_failure')` → **orphaned write**: QBO applied it but we didn't commit the success locally. Auto-advance the ledger row to `succeeded` and suppress the conflict. Prevents false-drift flagging after transport timeouts that actually landed on Intuit.
- Marker mismatch (even with recent timestamp) → potential drift, run full detector compare.
- Field-level intent compare (Phase 2 stale-retry guard + Phase 3 detector) both operate on `comparable_hash` from the normalized projection, not raw payload.

**Timestamp precision.** `result_last_updated_time` and `observation.last_updated_time` must use the same canonical precision. QBO returns ISO-8601 strings with variable precision; Postgres timestamptz stores microseconds. Ingest normalizes both to **millisecond-truncated UTC timestamptz** before storage so equality comparison does not silently fail on precision mismatch. Store the raw string in `payload` JSONB for audit but drive correlation off the normalized column.

## 8. Code changes — new vs extend

### Extends existing

- `modules/integrations/src/domain/oauth/{service,repo}.rs` — add `upsert_connection_by_app_provider`; `http/oauth.rs` callback requires `state`.
- `modules/integrations/src/domain/qbo/cdc.rs` — tombstone classification (currently no branch at line 241); advance watermark using provider-confirmed high-watermark from payload `MetaData.LastUpdatedTime`, not `Utc::now()`.
- `modules/integrations/src/domain/webhooks/routing.rs` — add delete webhook mappings (currently only created/updated).
- `modules/integrations/src/domain/qbo/client.rs` — add `create_customer`, `create_payment`, `void_invoice`, `delete_customer` (deactivate), `update_customer`, `update_payment`; honor `Retry-After` and quota headers on rate limits.
- `modules/integrations/src/domain/qbo/outbound.rs` — migration cutover fence: gate existing `spawn_order_ingested_consumer` / `spawn_outbound_consumer` on a feature flag + authority check, so pre-Stream-D flows don't double-push once authority lands (pre-existing consumers auto-start at `main.rs:96,130`).
- `modules/integrations/src/domain/qbo/sync.rs` — write into `integrations_sync_observations` instead of straight outbox.
- Invoice payload — add currency/tax/locale fields.

### New

- `modules/integrations/src/domain/sync/` — module tree:
  - `mod.rs`, `authority.rs`, `authority_repo.rs`, `conflicts.rs`, `conflicts_repo.rs`, `push_attempts.rs`, `resolve_service.rs`, `resolve_customer.rs`, `resolve_invoice.rs`, `resolve_payment.rs`, `dedupe.rs`, `observations.rs`.
- `modules/integrations/src/http/sync.rs` — REST handlers.
- `contracts/events/integrations.sync.*` — schemas for new events.
- Migrations:
  - `20260420000014_oauth_reconnect_upsert.sql` (Phase 0)
  - `20260420000015_create_sync_authority.sql`
  - `20260420000016_create_sync_push_attempts.sql`
  - `20260420000017_create_sync_conflicts.sql`
  - `20260420000018_create_sync_observations.sql`
  - `20260420000019_add_webhook_delete_routes.sql` (if webhook routing needs schema support)

### Dropped

- Trait-dispatched `push<T: QboEntity>` — explicit handlers instead.
- `service_account_label` column — Intuit doesn't support it.
- `integrations_sync_health` dedicated table — defer; prove query shapes via ledger aggregates + existing metrics.rs first.
- `push.succeeded` event — counters only.

## 9. Phasing

**Phase 0 — OAuth reconnect fix + state validation (prerequisite, standalone).**
- Upsert on callback; require state; tests. Unblocks disconnect → reconnect UX for Stream D.

**Phase 1 — Foundation.**
- Env wiring for QBO_* + OAUTH_ENCRYPTION_KEY in `docker-compose.services.yml` (James flips compose bypass).
- Generate OAUTH_ENCRYPTION_KEY (32 bytes).
- Align QBO_REDIRECT_URI with Intuit dev app registration.
- Normalize env var naming (`QBO_BASE_URL` canonical; remove `QBO_API_BASE` if present).
- New permissions in `platform/security/src/permissions.rs`.
- Migrations 15, 17 (authority registry + conflicts).
- `contracts/events/integrations.sync.*` stubs.
- Migration cutover fence on existing ad-hoc consumers (feature flag, default OFF until Phase 2 ships).

**Phase 1.5 — Ledger + fence token + inflight watchdog.**
- Migration 16 (push_attempts).
- Authority flip acquires advisory lock; bumps version; quiesces pending outbox rows for the entity_type.
- Ledger records every push intent at accept time; re-checks version at commit.
- **Inflight watchdog worker** — periodically scans `integrations_sync_push_attempts WHERE status='inflight' AND started_at < NOW() - INTERVAL '10 minutes'`, transitions them to `failed` with `error_code='inflight_timeout'`. Prevents stuck rows from permanently blocking the partial unique index. Watchdog interval: 60s. Timeout threshold: 10 minutes (longer than any legitimate QBO call).

**Phase 2 — Explicit entity handlers + field-level intent guard.**
- `create_customer`, `create_payment`, `void_invoice`, `delete_customer` (deactivate), `update_customer`, `update_payment` on QBO client (currency/locale aware).
- `resolve_customer.rs` / `resolve_invoice.rs` / `resolve_payment.rs` handlers.
- `POST /sync/push/{entity_type}` endpoints with full push-result taxonomy from §6.
- QBO fault taxonomy mapped to taxonomy codes (`classify_error` extension).
- `Retry-After` + quota headers honored on rate limits.
- **Field-level intent guard on stale-retry.** When QBO returns `StaleObject`, client re-fetches, then compares **only the caller's touched-field mask** against the fresh-fetched projection. Touched-field drift → `StaleObject` result + conflict row. Untouched-field drift → merge-safe retry with fresh SyncToken. System fields (SyncToken, MetaData, server-calculated totals/tax) excluded from compare to prevent false positives. If baseline projection is missing or ambiguous, fail conservative as `StaleObject`.
- Every handler writes result markers (`result_sync_token`, `result_last_updated_time`, `result_projection_hash`) to the ledger on success.
- **Transport-level idempotency via QBO `requestid` param.** Each push handler passes the ledger's deterministic `request_id` (UUID) as the `requestid` query param on QBO writes (existing `write_url()` in `qbo/client.rs` currently generates a fresh UUID per call — replace with ledger request_id). Retries of the same ledger row reuse the same `request_id` → Intuit's built-in idempotency detects duplicate and returns the original response without creating a second entity. Critical for surviving transport timeouts (504s).
- Tests hit real Intuit sandbox (creds already in `.env.qbo-sandbox`).

**Phase 3 — Observation layer + detectors + queue read + jobs health.**
- Migration 18 (observations — includes `projection_version` + `comparable_hash`).
- Migration adding `integrations_sync_jobs` table.
- Fingerprint library + normalized projection builders in `sync/dedupe.rs`.
- CDC writes into observations with comparable_hash; webhook normalizer schedules fetch → observation.
- Tombstone classification in CDC handler; delete webhook mappings in routing.
- Detector-ledger correlation using result markers (not time window) to suppress self-echo.
- Field-level compare in detector as defense-in-depth for whatever Phase 2 guard missed.
- **Duplicate remap policy** in `resolve_customer`: stale `external_ref` + new external entity appears → raise `creation` conflict. Allow deterministic candidate hints only (exact normalized email / phone / tax ID match); no name-similarity auto-remap. Resolution action explicitly tombstones old mapping before linking new.
- CDC watermark advances using provider-confirmed high-watermark (max `MetaData.LastUpdatedTime` in batch), not `Utc::now()`.
- Each non-push worker upserts its row in `integrations_sync_jobs` per tick.
- `GET /sync/conflicts` + filters. `GET /sync/jobs`.
- `integrations.sync.conflict.detected` emission.

**Phase 4 — Resolution API.**
- `POST /sync/conflicts/{id}/resolve` and `POST /sync/conflicts/bulk-resolve`.
- Deterministic idempotency key = `conflict_id + action + authority_version`.
- Bulk route via explicit match in `resolve_service.rs`.
- `integrations.sync.conflict.resolved` emission.

**Phase 5 — Production cutover (blocked on James).**
- Production Intuit app setup, scope review, production client_id/secret, redirect URI registered.
- Production env wiring.

## 10. Dependencies

Within platform:
- Phase 0 blocks everything with a disconnect/reconnect story (i.e., Stream D full).
- Phase 1 blocks 1.5, 2, 3, 4.
- Phase 1.5 blocks 2 (handlers record ledger at accept).
- Phase 2 blocks Phase 3's HP-side detector (detector needs ledger correlation to suppress in-flight legitimate writes from drift).
- Phase 3 blocks Phase 4.
- Phase 5 is orthogonal to 1–4.

Cross-repo (Huber consumes):
- After Phase 0 — can wire disconnect/reconnect UX.
- After Phase 1 — authority toggle UI (§4.2 in Huber plan).
- After Phase 2 — entity mapping refactor (§4.1 in Huber plan).
- After Phase 3 — conflict queue UI (§4.3) + sync event handling (§4.5).
- After Phase 4 — resolution actions.
- E2E cycle test needs all phases + one fixed kit external_item_id.

## 11. Out of scope

- Live Intuit production app approval (James — prerequisite to Phase 5 only).
- Any Huber-side UI.
- Other integration providers beyond QBO (future — same shape applies).
- `projection_refresh` / `nats_subscriber` health on Huber tenant — those are `crates/huber-power/src/qbo/` Huber-side code, not 7D Platform.
- Alerting (metrics.rs + Prometheus handle it; thresholds are ops).
- `service_account_label` per-vertical branding in QBO audit (Intuit doesn't support it; use separate client_ids if needed later).

## 12. Open items — none (after R1+R2+R3+R4 + Gemini review)

All adversarial-review findings resolved:
- In-flight flip semantics → version check with split outcomes: pre-call = `AuthoritySuperseded`; post-call = `CompletedUnderStaleAuthority` with auto-reconciliation (equivalent → auto-close, divergent → conflict row).
- Bulk resolve atomicity → server always computes deterministic key; caller key aliases.
- Phase ordering → Phase 1.5 inserted for ledger + fence; detector correlates via ledger result markers, not time window.
- Health surface → narrow `integrations_sync_jobs` table in Phase 3 for non-push workers. Ledger covers push. metrics.rs covers alerting.
- Authorization model → 4 new capabilities.
- Retry/DLQ after flip → outbox `failure_reason` column; `GET /sync/dlq` filters.
- Event naming → unversioned to match existing code.
- OAuth reconnect + state validation → Phase 0 standalone bead (upsert on callback + state CSRF check).
- Service-account label → dropped; Intuit has no per-request actor override mechanism.
- Trait dispatch → explicit per-entity handlers, routed via `resolve_service.rs` match.
- CDC/webhook overlap → canonical observations layer with fingerprint (+ fallback when SyncToken/LastUpdatedTime missing).
- Detector self-echo → ledger result markers (`result_sync_token`, `result_last_updated_time`, `result_projection_hash`) on successful pushes enable marker-equality suppression; 1-hour window only bounds query cost.
- Field-level intent guard → Phase 2 push-path compares caller's touched-field mask only, excludes system-calculated fields; Phase 3 detector is defense-in-depth.
- Duplicate customer remap → explicit creation conflict; deterministic candidate hints only (exact email/phone/tax ID); no name-similarity auto-remap; resolution tombstones old mapping.
- Deletion taxonomy → tombstone + delete webhook mapping in Phase 3.
- Currency/locale → Phase 2 payload extension.
- Multi-entity ordering → caller sequences (simpler); platform only sequences within bulk-resolve pulls.
- Push-result taxonomy → `ExternalNotFound`, `InternalError { retryable }`, `UnknownFailure`, `CompletedUnderStaleAuthority` added; ledger `aborted` renamed `completed_under_stale_authority`.
- Observation normalized projection → `projection_version` + `comparable_hash` columns; raw payload retained for audit only.
- CDC watermark → advances via provider-confirmed high-watermark (max `MetaData.LastUpdatedTime`), not `Utc::now()`.
- Inflight poison pill (Gemini) → watchdog worker transitions stuck `inflight` → `failed` after 10 minutes; keeps unique-index protection for legit in-flight.
- Disconnect realm_id lockout (Gemini) → partial unique `(provider, realm_id) WHERE connection_status='connected'` lets disconnected rows cease blocking.
- Orphaned QBO writes (Gemini) → detector-ledger correlation also matches `failed`/`unknown_failure` rows and auto-advances them to `succeeded` on marker match.
- QBO transport idempotency (Gemini) → push handlers pass ledger `request_id` as QBO `requestid` query param; retries reuse, Intuit dedupes.
- Timestamp precision (Gemini) → normalize to millisecond-truncated UTC timestamptz at ingest; raw string kept in payload for audit.

---

End.
