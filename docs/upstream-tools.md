# Upstream Tool Tracking

Tools from Jeffrey's repos (Dicklesworthstone). Track versions, patches, and upgrade notes here so we never lose a customization again.

## br (beads_rust)

| Field | Value |
|-------|-------|
| Repo | github.com/Dicklesworthstone/beads_rust |
| Current | 0.1.34 (upgraded 2026-03-28) |
| Latest | 0.1.34 |
| Binary | ~/.local/bin/br-real |
| Wrapper | scripts/br-wrapper.sh (injects --actor, --assignee, --lock-timeout; claim guard) |
| Patched? | No — all customization is in the wrapper, binary is stock |
| Status | **Current** — upgraded 2026-03-28 |

**Upgrade beads:** bd-3rtg7 through bd-srfgz (8-bead plan, completed 2026-03-28)

### br Upstream Changelog: v0.1.13 → v0.1.34

Every feature, fix, and change we'd pull in by upgrading.

#### v0.1.14 (2026-02-14) — Frankensqlite migration

- Full migration from rusqlite to frankensqlite (pure-Rust SQLite)
- Batch upsert, FTS5 search, migration framework
- Atomic claim guard with claim.exclusive config and IMMEDIATE transaction
- Show command now displays design, notes, acceptance_criteria, external_ref fields
- NothingToDo exit code for idempotent operations
- Sync preflight guardrails for JSONL import validation
- History subcommand enhanced with session timeline

#### v0.1.15 (2026-02-23) — Tag only

- agents --dry-run --json with dry_run/would_action fields
- GITHUB_TOKEN support for self-update
- Map Rust target triples to release asset names
- Mark children of deferred epics as blocked in ready cache
- MIT with OpenAI/Anthropic Rider license

#### v0.1.16–v0.1.18 (2026-02-23) — Tags only, CI fixes

- CI target installation fixes, version bumps, musl→gnu for GLIBC compatibility

#### v0.1.19 (2026-02-23) — CI stabilization

- Allow partial release, temporarily disable linux_arm64

#### v0.1.20 (2026-02-26)

- Draft status variant for pre-execution issues
- 6 community-reported fixes (#85, #86, #87, #88, #91, #92)
- macOS fsqlite VFS lock fix (c_short type mismatch)

#### v0.1.21 (2026-03-04) — Frankensqlite stabilization

- Official Claude Code skill for br
- Rust 2024 let-chains adopted
- Fix parallel write data loss from dead busy_timeout
- Blocked cache refresh after dep changes, cycle detection fix
- 4 bugs in rebuild_issues_table schema migration
- 5 community-reported bugs (#104–#108)
- Bump fsqlite fixing B-tree cursor and page-count header

#### v0.1.22 (2026-03-07) — Doctor/repair

- doctor --repair: rebuild DB from JSONL
- Automatic SQLite database recovery from JSONL export
- Windows/zip installer support
- -d, --parent, -e flags for br q
- Config prefix inference from JSONL
- Comprehensive error propagation and transactional imports
- Wire up --hard flag to actually purge issues from DB

#### v0.1.23 (2026-03-07)

- --db override respected across all subcommands
- Enhanced diff output for history, CLI help styling
- Remove non-functional musl binary in installer

#### v0.1.24 (2026-03-08)

- InheritedOutputMode for consistent output format propagation
- Enhanced dependency tree visualization with theming and quiet mode
- SQLite journal support, git context fixes
- Convergence-based blocked-cache propagation (replaces silent depth cap)
- SQL-aware statement splitter (replaces naive split(';'))

#### v0.1.25 (2026-03-11) — Dense subsystem release

- sync_equals() for semantic 3-way merge comparison
- Bidirectional dep traversal and improved cycle detection
- SyncConflict error to prevent silent data loss on auto-import
- Assignee defaults and stats overhaul
- Long/pretty output modes with box-drawing tree connectors
- Today/yesterday time keywords, DST-safe helpers
- Exclude in_progress from ready output
- Streaming hash update, to_writer with reusable buffer
- Fast-path SQL limit push-down
- External-ref uniqueness enforcement
- Many bug fixes (YAML parsing, label dedup, tombstone transitions, DST)

#### v0.1.26 (2026-03-11) — Cross-project routing first landing

- Cross-project issue routing with batched dispatch (show, blocked, ready, stats)
- Re-read JSONL before flush in no-db mode to prevent clobbering concurrent writes

#### v0.1.27 (2026-03-12) — Major architecture release

- Issue routing extended to all mutation commands
- TOON output support added to audit, lint, version, count, epic, stale, history, orphans, query
- Complete quiet mode across all commands
- Database family snapshot infrastructure with sidecar quarantine
- Automatic database recovery during issue mutation
- probe_issue_mutation_write_path() diagnostic helper
- Incremental blocked-cache updates with bulk cycle-check
- Deterministic export ordering, streaming git log
- Markdown file import with --parent and --dry-run

#### v0.1.28 (2026-03-13) — Stabilization

- Remove stale .rebuild-failed recovery artifacts from test fixtures

#### v0.1.29 (2026-03-18) — Major

**Performance**
- Frankensqlite upgraded to v0.1.1: ~100x write performance improvement (39f3e0e)

**New Capabilities**
- `br serve`: MCP server for direct AI agent integration (2195144, 8f35a53, 7a1c17a)
- TOON output format added to graph command (02c3bde)
- Closed-at consistency enforced in issue validation (0e805c4)
- Updated-before/updated-after filters for search_issues (f327da2)
- Default prefix changed from `bd` to `br` (e6e7dcb)
- `delete --hard` now properly purges issues from JSONL (e6e7dcb)

**Bug Fixes**
- Fix hyphenated issue ID prefix parsing via split_prefix_remainder (8fa3edf)
- Suppress human output for sync subcommands under --quiet (3c7961e)
- Orphans command manages its own JSONL freshness (6c7fb5d)
- Propagate subcommand --robot flag through OutputContext (3cb1741)
- Atomic config writes, empty-comment validation, MCP ID-check ordering (1796519)
- Unicode-width-aware truncation in dep tree (72b8560)
- Exclude deferred issues from --overdue listing (d4cff76)
- Exclude in_progress issues from ready work queue (f226f66)
- Auto-register ParentChild dependency during import when parent is resolved (1290385)
- Show full transitive cascade closure in delete dry-run preview (94c3486)

**Security**
- CSV formula injection mitigation and log permission error handling (ab5356d)
- Whitelist table/column pairs in has_missing_issue_reference (014e676)

**Storage Hardening**
- Harden schema and query paths for fsqlite compatibility (47fa201)
- Doctor: use typeof() instead of IS NULL for NULL detection (841c49b)
- Replace local path deps with git URLs in [patch.crates-io] (988d5c7)
- Fix schema default, _beads support, init env vars (758f895)
- Server-side unassigned filter in MCP instead of post-filtering (87cfaa4)
- Force-flush fix applied to CLI export path (6501dff)

#### v0.1.30 (2026-03-20) — Major

**New Capabilities**
- Mixed issue ID prefixes: projects can contain issues from multiple prefix namespaces (d012e19)
- Paginated JSON envelope for list output: {issues, total, limit, offset, has_more} (580d281, 3b46f33)
- Deferred blocked-cache refresh for dependency mutations (45232f6)
- Batched mutation commands with stale-cache pre-marking (cdd9cb4)
- Expanded stats command with many additional aggregate metrics (ac4ff74, 4703dff, b634768)
- Expanded blocked/count/stale/epic/lint commands (0987d6e, 3126725, c4f861c, 0333b98)
- Close command expanded with additional status transitions (0f4f094)
- Batched blocked-cache refresh with stale-marking fallback (afa8d06)

**Bug Fixes**
- Correct list offset after client-side filtering for correct pagination (36a5ff8)
- Resolve concurrent DB corruption false positives in doctor (3a1feef)
- Fix show --json jq accessor to use array index (0d0fc38)
- Only add unalias br when an actual alias definition exists (0b7b070)

#### v0.1.31 (2026-03-21) — Hardening

**Storage and Reliability**
- Atomic config writes using PID-scoped temp files (e3a00e3)
- Graceful missing-dependency fallback in storage and graph code paths (617572f, a1b63dd)
- Blocked-cache hardening: single-row inserts, deferred invalidation, INSERT OR REPLACE (ad27f47, acedf9d, f687166)
- Lazy config loading and reduced sync lock contention (a690d58)
- Ready-query/storage fast path: column-projected ready queries, compare-and-set claims (9550859)

**Sync and Concurrency**
- Best-effort JSONL witness refresh
- Auto-import SyncConflict downgraded to warning for concurrent multi-agent writes (4bc6681)
- Centralized ID resolution into resolve_issue_id(s) helpers (94c9138)
- Redundant index removal, simplified event inserts, dependency thread index (311225e)

**Diagnostics**
- Doctor warns when root .gitignore hides .beads/.gitignore (5f1da48)

#### v0.1.32 (2026-03-23) — Cross-project routing

**Cross-Project Routing**
- Route-aware dependency operations with auto-flush and cross-project guards (4682499)
- Graph, delete, audit log, and lint now respect external workspace routing (5a983bc, d63f56c, 4f232bb, d231bce, d4df28f)
- Auto-import propagation reaches all routing callsites (506b6cf, 911b793)

**Storage and Config Hardening**
- Prefix normalization through config, storage, and ID handling (bdc0243, 0575380)
- Frankensqlite compatibility: batched DELETE replaced with row-by-row queries (ba71494, b9a0f25, 45b2a4e)
- Tombstone state handling: closed_at separate from deleted_at (new)
- Doctor improvements for root .gitignore conflicts (44d47e6, e6ef576)

#### v0.1.33 (2026-03-23) — Release CI

- Rust cache pinning updated to Swatinem/rust-cache v2.9.1
- Release builds fail closed on missing artifacts
- Cross-platform fallback coverage improved for Linux ARM64 and Windows AMD64
- Single-issue graph rendering preserves DFS subtree order

#### v0.1.34 (2026-03-24) — Hardening

- Thread dep_type through would_create_cycle signature (a0f5328)
- Harden unicode char counting, tombstone guards, path traversal, and cycle detection (c021de8)
- Proptest regression file for ID property tests (6feb8dc)
- MinGW libKernel32.a case-sensitivity symlink for Windows cross-compilation (2760166)

### br-wrapper.sh Changelog

#### 2026-03-30 — Hard block on ownerless in-progress beads (bd-vi9da)

- `br update --status in_progress` without `--assignee` is now BLOCKED when agent identity cannot be resolved. Previously the auto-inject silently skipped, allowing beads to enter in_progress with no owner.
- Detection of `_is_claim` and `_has_assignee` moved outside the `_resolved` conditional so the guard fires regardless of identity resolution.
- Audit line emitted: `gate=claim decision=blocked reason=no_assignee_no_identity`.
- Explicit `--assignee` still works even without identity resolution (e.g. orchestrator reassigning).

## bv (beads_viewer)

| Field | Value |
|-------|-------|
| Repo | github.com/Dicklesworthstone/beads_viewer |
| Current | 0.15.2 (upgraded 2026-03-28, assignee patch LOST — needs re-apply) |
| Latest | 0.15.2 |
| Binary | ~/.local/bin/bv |
| Upstream backup | ~/.local/bin/bv-upstream (0.14.4 stock) |
| Stock backup | ~/.local/bin/bv.backup |
| Patched? | **Yes — binary was rebuilt from source** |
| Status | **v0.15.2 — upgraded, assignee patch needs re-apply from source** |

**Known patches (rebuilt from source at /tmp/beads_viewer_build):**
1. **Assignee replaces age in TUI list** — `pkg/ui/delegate.go:71-79`: when `i.Issue.Assignee != ""`, show truncated assignee name instead of "1h ago" in the right column. Removed the separate assignee column at width>100 (redundant).
2. **Assignee in HTML export viewer** — `pkg/export/viewer_assets/index.html`: 3 locations changed `formatDate(issue.updated_at)` to `issue.assignee || formatDate(issue.updated_at)`.
3. **Schema fix for br 0.1.34** — `internal/datasource/sqlite.go`: `i.due_date` → `i.due_at`, `tombstone IS NULL OR tombstone = 0` → `deleted_at IS NULL` (2 locations: main query and simple fallback). Also added `assignee` to the simple fallback SELECT and Scan.
4. **Prefix references** — now moot, upstream did bd→br migration.

**Before updating:** Check if 0.15.2 includes assignee display natively. If not, clone the beads_viewer repo, find the Alpine.js template (likely in an embedded HTML file), re-apply the assignee patch, and rebuild with `go build`.

### bv Upstream Changelog: v0.14.4 → v0.15.2

Every feature, fix, and change we'd pull in by upgrading.

#### v0.15.0 (2026-03-08) — Major release

**Compatibility & Data Source**
- Read labels from separate `labels` table for br/beads-rs SQLite compatibility (19437c4)
- `--db` flag and `BEADS_DB` env var for configuring database path (b56ddae)
- Migrate from Go `flag` to `pflag` for POSIX double-dash options (064b3d0)
- Complete bd-to-br command migration across all source and tests (f9ba482, 6bce598)

**Status & Display**
- Color mappings for deferred, draft, pinned, hooked, review, and tombstone statuses (42d69f7, ce542b3)
- Footer text contrast fix across terminal themes (271cb10)
- Color-profile-aware styling for Solarized and 16-color terminals (cbbcb1f)
- Terminal default background to prevent ANSI color mismap (2599cce)

**Security**
- Scope GitHub token to github.com domains to prevent credential leaking on redirects (ccd23d0)
- Trim whitespace from GitHub token env vars to prevent 401 errors (a148823)
- GITHUB_TOKEN support for self-update (2ff6cab)

**Board View**
- Column width calculation fix to prevent line rendering glitch (08eb523)
- Board columns can shrink below 12 chars on narrow terminals (e50be8a)
- Detail panel box drawing correction (2ff6cab)

**Robot Mode & Agent Support**
- `--agents-*` CLI flags for AGENTS.md blurb management (8e9c656)
- Normalized robot output envelope across all commands (23172a1)
- Recipe filtering applied before robot modes (dc6bfab)
- Agent blurb upgraded to v2 (ce542b3)

**Version Detection**
- Multi-source version detection with graceful fallback (ede65f2)
- Filter pseudo-versions and dirty builds from version detection (1bb3c27)
- Validate ldflags injection to prevent empty version output (087af33)

**Triage & Analysis**
- Transitive parent-blocked check in GetActionableIssues (b14e9c4)
- Invalidate robot triage disk cache when .beads/ directory changes (9464db4)
- Deep mtime scan via WalkDir for cache invalidation (d1e8233)

**Deployment**
- GitHub Pages and Cloudflare deployment support for static export (e60384b)

**License**
- Updated to MIT with OpenAI/Anthropic Rider (81c2b94)

#### v0.15.1 (2026-03-09) — Patch

- Fix wrangler auth check hanging on headless servers (4cc8635)

#### v0.15.2 (2026-03-09) — Patch

- Check all wrangler config paths and handle refresh tokens (cf001ba)

#### Upstream (unreleased, on main)

- Smart terminal editor dispatch via `O` key for opening beads in $EDITOR (550f3bd)
- YAML frontmatter: single-pass unescape, escape all fields, fix body whitespace and labels (7a481b0, 90aa46e, c0f670b)
- Preserve issue deep-links during cold load filter sync (81a1983)
- Guard truncate() against negative/small max and nil Process check (816f9c3)
- Guard against negative strings.Repeat and normalize whitespace status (a0a35ee)

#### What's NOT upstream (our patches to re-apply)

1. Assignee display in TUI list items — 3 Alpine.js template locations
2. Prefix text changes (now moot — upstream did bd→br migration)

## cm (cass_memory)

| Field | Value |
|-------|-------|
| Repo | github.com/Dicklesworthstone/cass_memory_system |
| Current | 0.2.3 |
| Latest | 0.2.3 |
| Binary | ~/.local/bin/cm |
| Patched? | Unknown |
| Status | **Current** — no update needed |

## cass (coding_agent_session_search)

| Field | Value |
|-------|-------|
| Repo | github.com/Dicklesworthstone/coding_agent_session_search |
| Current | 0.2.4 (upgraded 2026-03-29) |
| Latest | 0.2.4 |
| Binary | ~/.local/bin/cass |
| Patched? | No — stock binary |
| Status | **Current** |

### cass Upstream Changelog: v0.2.0 → v0.2.4

#### v0.2.1 (2026-03-09)

- Kimi Code and Qwen Code connector stubs
- Copilot CLI connector module
- Incremental embedding in watch mode — semantic index updates as new sessions arrive
- Colorblind theme preset for deuteranopia/protanopia
- Statically link OpenSSL to eliminate libssl.so.3 runtime dependency
- TUI resize logging opt-in to prevent disk exhaustion
- Include "tool" role messages in all export formats
- health --json now reports real DB stats

#### v0.2.2 (2026-03-15)

**Security**
- Secret redaction: secrets detected in tool-result content are redacted before DB insert

**Storage**
- FTS5 on FrankenSQLite with doctor diagnostics fixes
- Chunked FTS rebuild to prevent OOM
- Replace sqlite_master queries with direct table probes

**Safety**
- Replace unwrap calls with safe error handling across search, export, timeline, tests
- Null-safety guards in router, service worker, perf tests

**UI**
- Colorblind theme redesign for deuteranopia/protanopia
- Missing-subcommand hints for CLI

#### v0.2.3 (2026-03-24)

**Search and indexing**
- FTS5 contentless mode (schema V14): reduced DB size while preserving query performance
- LRU embedding cache: avoids redundant ONNX inference
- Expanded query pipeline with progressive search integration
- NaN-safe score normalization
- Penalize unrefined documents in two-tier blended scoring
- Parallel indexing: multiple connector sources concurrently

**TUI**
- HTML/PDF export pipeline rewrite with improved layout
- TUI search overhaul with improved result rendering
- Analytics dashboard expansion: additional chart types, structured error tracking
- Click-to-position cursor in search bar
- UltraWide breakpoint for ultra-wide terminals
- Search-as-you-type supersedes in-flight requests

**Health and storage**
- WAL corruption detection with degraded health state reporting

**Export**
- Skill injection stripping from HTML, Markdown, text, JSON exports
- Legible code blocks without CDN dependencies

**Dependencies**
- Complete migration from rusqlite to frankensqlite
- Reqwest removed, HTTP calls migrated to asupersync

#### v0.2.4 (2026-03-27)

**Bug fixes**
- INSERT...SELECT UPSERT/RETURNING fallback for frankensqlite compatibility
- Cross-database rowid watermark fix
- Auto-repair missing analytics tables
- FrankenStorage connection handling: explicitly close all connections
- Suppress frankensqlite internal telemetry in default log filter

**New features**
- Historical session recovery toolkit
- Database health integration: quick_check, FTS consistency repair
- Crush connector from franken_agent_detection
- Resumable lexical rebuild with durable checkpoints
- Seed canonical DB from best historical bundle via VACUUM INTO

**Performance**
- Replace COUNT(*) rebuild fingerprint with fs stat
- Batch message fetching and multi-threshold commit triggers
- Restructure daily stats rebuild

## fsfs (frankensearch)

| Field | Value |
|-------|-------|
| Repo | github.com/Dicklesworthstone/frankensearch |
| Current | 1.2.5 (upgraded 2026-04-10) |
| Latest | 1.2.5 |
| Binary | ~/.local/bin/fsfs |
| Backup | ~/.local/bin/fsfs-1.1.7 (auto-created by fsfs update) |
| Patched? | No — stock binary |
| Status | **Current** — upgraded 2026-04-10 |

### fsfs Upstream Changelog: v0.1.0 → v1.2.5

#### v1.0.0 (2026-02-21) — First stable release

- Query canonicalization pipeline rewrite with fastembed cross-encoder reranker
- Multi-model ONNX embedder support via OnnxEmbedderConfig
- VDBE sqlite_master parameterized query workaround
- Daemon module fixes, exclude pattern suffix matching
- Windows build portability and installer checksum fallback

#### v1.1.0 (2026-02-22) — Crates.io, resilient indexing, Apple Silicon

- All workspace dependencies switched from local paths to crates.io
- Checkpoint resume, embedding retries, degraded-mode completion
- Symlink cycle prevention in snapshot walker
- Fix nested markdown links, stabilize MMR, add bounds checks
- Identifier detection improvements, WAL-first lookups, score normalization
- macOS arm64 (Apple Silicon) as first-class release target

#### v1.1.1 (2026-02-22) — Hotfix

- Fix first-run hang on macOS (reduced filesystem probe budget)
- Progress indicator during initial filesystem scan
- SHA256SUMS filename fix

#### v1.1.2 (2026-02-22) — Version and update fixes

- Binary now correctly reports its version (was stuck at v0.1.0)
- fsfs update constructs correct download URLs
- SHA256SUMS download for update verification
- Windows target triple detection fix
- TUI prints diagnostic messages on exit
- HashSet for O(1) duplicate doc_id detection

#### v1.1.3 (2026-02-23) — PDF extraction, lite builds

- Native PDF text extraction — fsfs index/search can process PDFs directly
- embedded-models feature flag for lite/offline builds
- Rank movement explanations in TwoTierSearcher
- Beautiful download progress with file-size display
- 6 security/correctness fixes in installer and update logic
- 7 code-review bug fixes

#### v1.1.4 (2026-03-22) — Cloud API embeddings, WAL mutations

**Cloud API Embedding Providers**
- Pluggable cloud API embedding: OpenAI and Gemini backends
- HTTP transport, automatic retry, token-bucket rate limiting, L2 normalization
- Query-param authentication for Gemini

**Tokenizer & Search**
- Preserve hyphenated bead IDs (e.g. bd-q3fy) in cass tokenizer (schema v6→v7)
- Regex tokenizer fix: [a-zA-Z0-9] instead of \w to exclude underscores

**In-Memory Vector Index**
- Fully-resident in-memory vector index with f16 quantization
- Synchronous two-tier search API

**WAL-Based Incremental Mutations**
- append-batch, delete, compact, daemon commands for WAL-based incremental index mutation

#### v1.1.5 (2026-03-22) — Tag only

- Fix double score calibration of fast-tier semantic hits
- Replace non-existent PolledAfterCompletion with wildcard match in embed/lexical/rerank
- Compilation fixes

#### v1.1.6 (2026-03-23) — Tag only

- Make sidecar and durable writes atomic, remove unsafe pre-rename delete
- Only transition to Recovering phase from Bootstrap on Replay work
- Deduplicate main+WAL results and unify NaN score handling
- Make WAL cleanup best-effort, add temp-file cleanup on rewrite error
- Use doc_id strings for search dedup, fix rewrite error path
- Bump fsqlite deps to 0.1.1 then 0.1.2 across crates
- Adapt to fsqlite 0.1.2 API changes (Arc<str>/Arc<[u8]> values, Cx type)
- Eliminate checksum ambiguity in InteractionSnapshot::compute_checksum

#### v1.1.7 (2026-03-23) — Latest

- Resolve CI warnings-as-errors and OpenSSL cross-compilation failure
- Allow partial release publish when some targets lack ORT prebuilts

## mcp_agent_mail

| Field | Value |
|-------|-------|
| Repo | github.com/Dicklesworthstone/mcp_agent_mail |
| Current | 0.3.0 (vendored in mcp_agent_mail/) |
| Latest release | 0.3.0 |
| Upstream main | **217 commits ahead of v0.3.0** (as of 2026-03-28) |
| Binary | mcp_agent_mail/.venv/bin/python3 -m mcp_agent_mail.cli serve-http |
| Patched? | Unknown — vendored copy, may have local changes |
| Status | **Significant unreleased upstream work** |

### mcp_agent_mail Upstream Changelog: v0.3.0 → main (unreleased)

217 commits since v0.3.0, spanning 2026-01-07 through 2026-03-28. (CHANGELOG.md says 129 but was written mid-development; gh compare shows 217.)

**Agent Identity and Lifecycle**
- Persistent window-based agent identity tied to terminal pane, surviving restarts
- Canonical per-pane agent identity file contract
- Sender identity verification and safe defaults
- Agent retire and project archive soft-delete (#102, #103)
- Hard delete with "I UNDERSTAND" confirmation (#105)

**Messaging and Coordination**
- Broadcast and topic threads for all-agent visibility
- On-demand project-wide message summarization
- Contact enforcement optimization with batch queries
- TOON output format support

**Server and Transport**
- /mcp endpoint alias alongside /api
- Periodic FD health monitor for file descriptor exhaustion
- Server launcher delegates to Rust `am` binary
- ExpectedErrorFilter for clean operational error logging

**Reservations and Storage**
- Virtual namespace support for tool/resource reservations
- Commit queue and archive locking for high-concurrency
- Git index.lock retry with exponential backoff

**Security**
- Constant-time bearer token comparison
- Localhost bypass prevention behind reverse proxies
- Gate absolute attachment paths (path traversal prevention)
- Remove ack_required bypass in contact policy

**Bug Fixes (many)**
- AsyncFileLock FD leaks
- Lightweight /api/health bypass when MCP layer saturated
- LIKE escape character ambiguity (switch to ! as escape)
- Missing schema migrations for registration_token and topic columns
- Cross-project git_paths_removed leak, XSS in confirmation dialog
- OAuth metadata path normalization
- Identity race condition and redundant DB lookups
- CLI exit hang, connection leak, SQLAlchemy GC warnings
- Thread digest corruption fix
- Search session leak in LIKE fallback

**Installer**
- Robust TOML URL upsert, Codex integration exports
- Avoid nested curl|bash for br/bv installation
- Settings merge instead of overwrite (#76)

## ntm (Named Tmux Manager)

| Field | Value |
|-------|-------|
| Repo | github.com/Dicklesworthstone/ntm |
| Current | v1.10.0 (upgraded 2026-03-29 via `ntm upgrade`) |
| Latest release | v1.10.0 (2026-03-25) |
| Upstream main | **41 commits ahead of v1.10.0** (as of 2026-03-28) |
| Binary | /opt/homebrew/bin/ntm (official release binary) |
| Patched? | **No — stock release binary. spawn-fix.go.patch NOT applied (see below)** |
| Status | **Current** — upgraded 2026-03-29 |

**Lost patch:** `ntm/spawn-fix.go.patch` reorders Agent Mail registration to happen BEFORE SendKeys during `ntm spawn`, so identity files exist before the agent process starts. This patch was in the dirty local build but is NOT in v1.10.0. The patch file is preserved at `ntm/spawn-fix.go.patch` and the old binary at `ntm/ntm-backup-dirty`. A new bead should be created to either re-apply this patch from source or verify upstream has incorporated it.

### ntm Upstream Changelog: v1.7.0 → v1.10.0+

#### v1.7.0 (2026-02-02)

**Privacy and Redaction**
- Redaction engine with PII detection, priority-sorted overlap dedup
- Privacy mode with config and CLI flags
- ntm scrub command for outbound notification/webhook redaction
- Safety profiles and robot support bundle flag

**Prompt Preflight and Linting**
- ntm preflight command for prompt validation
- Core lint rules and PII detection checkers
- DCG check integration in preflight

**Support Bundle**
- ntm support-bundle for diagnostic data collection
- Manifest schema and verification

**Agent Ecosystem**
- Ollama recognized as agent type
- spawn --local/--ollama for local Ollama agents
- Webhook formatters for Slack, Discord, Teams

**TUI and Dashboard**
- Smart animation with adaptive tick rate
- Cost panel, history panel with filtering/copy/replay
- Context usage and token counts in AgentStatus

#### v1.8.0 (2026-03-07)

**Session Labeling**
- --label flag for spawn, create, quick commands
- --project flag for send, kill, list
- ntm scale command for manual fleet scaling

**Encryption and Audit**
- Encryption at rest for prompt history and event logs
- ntm audit subcommands for log query and verification

**Ollama Model Management**
- Pull progress streaming and model deletion
- Local fallback with provider selection in spawn

**Monitoring**
- Prometheus exposition format export
- Expanded webhook payload templates
- Effectiveness tracking module and dashboard panel

**Rate Limiting**
- Codex rate-limit detection with AIMD adaptive throttling
- PID-based liveness checks replacing text-based detection
- Auto-restart-stuck agent detection

**Robot API Expansion**
- Major robot API infrastructure expansion
- SLB robot bridge, GIIL fetch wrapper
- --robot-output-format alias

**Bug Fixes**
- Claude Code idle detection overhaul (false positive reduction)
- Tmux buffer-based delivery for multi-line prompts
- Default Claude model updated to Opus 4.6
- Pipeline data race on state.UpdatedAt
- Integer overflow guard in backoff delay

#### v1.9.0 (2026-03-24)

**Agent Type Expansion**
- Cursor, Windsurf, Aider, Ollama support across all subsystems (CLI, robot, TUI, E2E)
- --robot-overlay for agent-initiated human handoff
- --attention-cursor flag for dashboard and overlay commands

**Attention Feed**
- Canonical flag names and robot-inspect-coordination action
- Session agent type counts

**Checkpoint**
- Restore subcommand with dry-run and context injection

**CLI**
- Session name resolvers, shared flag resolution across all robot commands
- Unified --since/--type as shared flags
- Assign watch overlay auto-binding, mouse support

**Dashboard**
- RFC3339Nano timestamp parsing, sidebar attention panel
- Pane delegate foundation for bubbles/list
- Spinner and progress bars wired to centralized animation detection

**Config**
- context_warning_threshold and project-scoped alert overrides

#### v1.10.0 (2026-03-25) — Latest release

**Config System**
- Unified retry policy and routing config sections
- Wire Print, GetValue, Diff, Validate for remaining config sections
- Project path resolution consolidation

**Models**
- Canonical model registry as single source of truth

**Robot Mode**
- Config-driven routing, rate limit detection, blocked beads
- Consolidate mail actions into robot-mail-check, add ack_required field
- Overhaul robot mail-check

**Bug Fixes**
- Clone context maps, stabilize ordering in alerts
- Defensive cloning and snapshot semantics for assignment store
- Kill monitor in buildKillResponse
- Redirect unsafe-palette warning from stdout to stderr
- Propagate ProjectKey through mail inbox summary
- Rune-safe string ops, validate hardening, tmux alias cleanup
- Data race guards, nil-safe handoff in rotation
- Atomic counter for tmux buffer names (prevent concurrent collisions)

#### Unreleased (after v1.10.0)

**TUI Overhaul ("Glamour Upgrade")** — from the v1.8.0→v1.9.0 development cycle:
- Vendored Bubbletea fork with theme system overhaul
- Spring animations engine for progress bars, focus transitions
- Spawn wizard with gradient tab bar
- Scrollable panels with toast system
- Charmbracelet/huh forms for interactive dialogs
- Help overlay with bubbles/help FullHelp and Catppuccin theming
- 6-panel mega layout with dedicated attention column

**Server and API**
- Full WebSocket hub with REST API handlers
- Operator loop guardrails and REST/CLI parity

**Stability**
- Panic recovery in goroutines, closed-channel drain prevention
- UTF-8 truncation, escape parsing, OOM protection
- Tmux pane ID format fix (session:N → session:.N)
- Goroutine leak fixes in swarm and webhook
- ~1k lines dead code removed

## ru (repo_updater)

| Field | Value |
|-------|-------|
| Repo | github.com/Dicklesworthstone/repo_updater |
| Current | 1.2.1 |
| Latest | 1.2.1 |
| Binary | ~/.local/bin/ru |
| Patched? | Unknown |
| Status | **Current** — no update needed |

## markdown_web_browser

| Field | Value |
|-------|-------|
| Repo | github.com/Dicklesworthstone/markdown_web_browser |
| Current | 0.1.0 (in markdown_web_browser/) |
| Latest | No releases found |
| Patched? | Unknown |
| Status | Unknown — no upstream releases to compare against |

## ubs (ultimate_bug_scanner)

| Field | Value |
|-------|-------|
| Repo | github.com/Dicklesworthstone/ultimate_bug_scanner |
| Current | v5.0.7 |
| Latest | v5.0.7 |
| Binary | Meta-runner (Python-based, in ultimate_bug_scanner/) |
| Patched? | Unknown |
| Status | **Current** — no update needed |

## dcg (destructive_command_guard)

| Field | Value |
|-------|-------|
| Repo | github.com/Dicklesworthstone/destructive_command_guard |
| Current | v0.4.0 |
| Latest release | **v0.4.0** (with binaries; v0.4.3 tag-only, unreleased work beyond) |
| Binary | ~/.local/bin/dcg |
| Patched? | No |
| Status | **Current** — on latest release with binaries |

### dcg Upstream Changelog: v0.2.15 → v0.4.3+

#### v0.3.0 (2026-02-02) — Major

**Robot Mode**
- Structured JSON output and machine-readable exit codes (dcg test --robot)
- Schema versioning and metadata in TestOutput JSON

**Rich Terminal Output**
- rich_rust integration with DcgConsole wrapper
- Enhanced doctor, packs, stats with rich output
- Tree visualization for dcg explain

**Pack System Expansion**
- Detailed explanations for all destructive patterns
- External pack loading from custom_paths
- Expanded system.disk pack (mdadm, btrfs, LVM, dmsetup)

**Agent Profiles**
- Agent-specific profiles and trust levels — auto-detect AI coding agent

**Golden Testing**
- Golden JSON tests framework for deterministic output validation

#### v0.4.0 (2026-02-10) — Major

- GitHub Copilot CLI hook support and installer integration
- Timeout protection for agent scanning during install
- repository_dispatch triggers for homebrew/scoop automated packaging
- Evaluator refactored to consolidate external pack checking

#### v0.4.1 (2026-02-22) — Tag

- musl-based statically linked Linux binaries
- fsqlite and rich_rust deps switched to crates.io
- dcg pack-info shows patterns by default (--json, --no-patterns flags)
- Binary content detection for Unicode, FTS rowid sync, regex engine fallback

#### v0.4.2 (2026-02-23) — Tag

- Resolved 91+ pre-existing test failures
- License updated to MIT with OpenAI/Anthropic Rider

#### v0.4.3 (2026-03-14) — Tag

**Self-Healing**
- Real-time settings.json overwrite detection and self-healing
- dcg setup command with shell startup hook-removal detection
- Shell startup check for silently removed DCG hooks

**New Protection Packs**
- Supabase database protection pack (db push, db reset, migration repair, functions delete, etc.)

**Agent Detection**
- Gemini CLI hook protocol support
- Augment Code agent detection
- GitHub Copilot CLI agent detection

**Interactive Allowlist**
- Session-scoped allowlist with collision-resistant backups
- SQLite schema v6 migration

**TOON output** format support

#### Unreleased (after v0.4.3)

- Strict git pack: expanded dangerous-command detection
- Removed safe patterns creating compound-command bypass
- Podman rm/rmi combined-flag bypass (e.g. podman rm -af)
- Disambiguate Claude Code from Gemini in detect_protocol()

## jsm / acfs (Jeffrey's Skills Manager / Agentic Coding Flywheel Setup)

| Field | Value |
|-------|-------|
| Repo | github.com/Dicklesworthstone/agentic_coding_flywheel_setup |
| Current | jsm 0.1.5 |
| Latest release | v0.6.0 (2026-02-02) |
| Upstream main | **427+ commits ahead of v0.6.0** (internal version bumped to 0.7.0) |
| Binary | ~/.local/bin/jsm |
| Patched? | Unknown |
| Status | **5 release versions behind + 427 unreleased commits — skills delivery system** |

### jsm/acfs Upstream Changelog: v0.1.5 → v0.6.0+

Note: jsm is the CLI binary installed by ACFS. ACFS is the installer/manager framework. Versions track the ACFS repo.

#### v0.4.0 (2026-01-08) — Expanded Flywheel Stack

- 10 tools + utilities integrated
- Skills install/uninstall/list infrastructure

#### v0.5.0 (2026-01-11) — DCG & RU Integration

- DCG and RU integrated into the flywheel stack
- Expanded tool installation and management

#### v0.6.0 (2026-02-02) — Complete bd→br migration

**Binary Rename**
- Complete bd to br migration across entire project
- Removed stale alias br='bun run' from older ACFS versions
- CLI flags renamed: --no-bd to --no-br

**Expanded Tool Ecosystem**
- 5 new tools: APR, JFP, Process Triage, X Archive Search, Meta Skill
- SRPS (System Resource Protection Script) added
- WezTerm Automata and Brenner Bot lessons and tests
- Comprehensive TL;DR page showcasing all flywheel tools

**Installer Hardening**
- Crash-safe change recording and undo system
- --pin-ref flag for pinned installations
- Pre-flight warning auto-fix system
- Shell completion scripts for bash and zsh
- ACFS self-update as first operation in update command
- acfs doctor --fix and --dry-run modes
- acfs status one-line health summary

#### Unreleased (after v0.6.0, 427+ commits, internal v0.7.0)

**Installer & CLI**
- acfs services command for unified daemon management
- --only and --only-phase flags for selective installation
- --stack-only flag for acfs update
- Verified installer framework with 13 new tool installers
- 8 new stack tools integrated
- Curl timeouts to prevent indefinite hangs

**Agent Mail**
- MCP Agent Mail as systemd managed service (replaces tmux-spawn)
- Switched from Python to Rust installer

**Manifest & Security**
- Pre-install checks, extended drift detection
- Internal script integrity verification at install time
- GitHub Actions hardened against script injection
- Shell scripts hardened against unsafe inputs

**Testing**
- Expanded E2E, unit, VM tests
- Comprehensive tests for 9 new Dicklesworthstone tools

**Infrastructure**
- Systemd timer for daily unattended acfs-update
- ntfy.sh push notifications for agent task lifecycle

## slb (Simultaneous Launch Button)

| Field | Value |
|-------|-------|
| Repo | github.com/Dicklesworthstone/slb |
| Current | v0.2.0 |
| Latest | v0.2.0 |
| Binary | /opt/homebrew/bin/slb |
| Patched? | Unknown |
| Status | **Current** — no update needed |

## wa (wezterm_automata / frankenterm)

| Field | Value |
|-------|-------|
| Repo | github.com/Dicklesworthstone/frankenterm |
| Current | 0.1.0 (dirty local build) |
| Latest | No releases (repo last pushed 2026-03-28) |
| Binary | ~/.cargo/bin/wa |
| Patched? | Local build from source |
| Status | Unknown — no upstream releases to compare |

## flywheel_tools (@agentcore/flywheel-tools)

| Field | Value |
|-------|-------|
| Repo | Part of ACFS (github.com/Dicklesworthstone/agentic_coding_flywheel_setup) — no standalone repo |
| Current | 0.1.0 (local npm package symlinked via file:flywheel_tools) |
| Latest | Distributed via ACFS — updates come through acfs update |
| Binary | node_modules/.bin/flywheel (CLI dispatcher) |
| Patched? | **Yes — heavily modified locally** (agent-runner, hook-server, mail helpers, adapters) |
| Status | **Core framework — local modifications diverge from ACFS upstream** |

This is the main agent coordination framework: agent-runner.sh, hook scripts, bead management scripts, mail helpers, browser workers, adapters (deepseek, grok). It ships as part of ACFS but we maintain a heavily modified local copy. Changes we make here (like the br upgrade JSON parsing fixes) are local only and would be overwritten by an ACFS update.

**Before any ACFS update:** Diff our flywheel_tools/ against the ACFS version to identify local modifications that need to be preserved.

## flywheel_connectors

| Field | Value |
|-------|-------|
| Repo | github.com/Dicklesworthstone/flywheel_connectors |
| Current | Vendored in flywheel_connectors/ |
| Latest | No releases |
| Patched? | Unknown |
| Status | No upstream releases — vendored copy only |

---

## Policy

When modifying any upstream tool:
1. Save the stock binary as `<tool>-upstream` before patching
2. Document the change here with: what was changed, why, and which source files
3. Before updating, check if upstream now includes our patch
4. After updating, verify our patch is preserved or re-apply

When upgrading any upstream tool:
1. Document the full changelog for every version between current and target BEFORE upgrading
2. Run integration audit: map every place the tool is called and what output format is expected
3. Create beads with adversarial review before executing
4. **Test across ALL projects, not just AgentCore.** Global binaries (br, bv, cm, cass, fsfs, dcg, ntm, ru, slb) affect every spoke project. Schema changes in one tool can break another tool's reader (e.g. br 0.1.34 schema broke bv 0.14.4 TUI).
5. Upgrade dependent tools together when there are schema/format dependencies (br + bv must move in lockstep)

Active projects to verify after any upgrade:
- AgentCore (hub)
- Huber Power (spoke)
- Fireproof-ERP (spoke)
- 7D-Solutions Platform (spoke)
- Any other project in ~/Projects with a .beads/ directory
