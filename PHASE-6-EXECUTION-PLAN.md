# Phase 6 Execution Plan

Created: 2026-02-12
Agent: FuchsiaGrove

## 🎯 Completion Criteria

System is "Phase 6 Complete" when:
- No direct module imports
- No cross-DB access
- All events flow through EventBus
- Idempotency enforced
- Happy path passes end-to-end
- All contracts remain unchanged
- CI green

---

## 📋 Beads Created

### Stage 1: Infrastructure (Parallel Safe)

| Bead ID | Title | Status | Can Start? |
|---------|-------|--------|------------|
| bd-2b4 | 5.1a: AR module outbox/inbox infrastructure | ○ open | ✅ YES |
| bd-2hs | 5.1b: Payments module outbox/inbox infrastructure | ○ open | ✅ YES |
| bd-2go | 5.1c: Subscriptions module outbox/inbox infrastructure | ○ open | ✅ YES |
| bd-vwb | 5.1d: Notifications module outbox/inbox infrastructure | ○ open | ✅ YES |

**Gate:** All modules compile and run with `BUS_TYPE=inmemory`

### Stage 2: Documentation (Parallel Safe)

| Bead ID | Title | Status | Can Start? |
|---------|-------|--------|------------|
| bd-1yf | 5.2: Event subscriptions documentation | ○ open | ✅ YES |

**Gate:** Documentation complete, no code impact

### Stage 3: Validation (After Stage 1)

| Bead ID | Title | Status | Can Start? |
|---------|-------|--------|------------|
| bd-2yq | 5.3: Add envelope validation to all modules | ○ open | After Stage 1 |

**Gate:** All modules compile + tests pass

### Stage 4: Vertical Slice (Sequential)

| Bead ID | Title | Blocks | Blocked By |
|---------|-------|--------|------------|
| bd-2x8 | 6.1: AR emits collection command | bd-1h0, bd-mv0 | bd-2b4 |
| bd-1h0 | 6.2: Payments consumes and emits result | bd-2nh, bd-mv0 | bd-2hs, bd-2x8 |
| bd-2nh | 6.3: AR applies payment | bd-mv0 | bd-2b4, bd-1h0 |
| bd-3fm | 6.4: Notifications react to events | bd-mv0 | bd-vwb |
| bd-346 | 6.5: Subscriptions bill-run integration | bd-mv0 | bd-2go |

**Critical Path:** 6.1 → 6.2 → 6.3

### Stage 5: E2E Proof

| Bead ID | Title | Blocked By |
|---------|-------|------------|
| bd-mv0 | 6.6: End-to-end proof test | bd-2x8, bd-1h0, bd-2nh, bd-3fm, bd-346 |

**Gate:** Full happy path passes with BUS_TYPE=inmemory and optionally NATS

---

## 🚀 Parallel Execution Strategy

### Phase A: Infrastructure (Parallel - 4 instances)
```bash
# Instance 1
bv claim bd-2b4  # AR infrastructure

# Instance 2
bv claim bd-2hs  # Payments infrastructure

# Instance 3
bv claim bd-2go  # Subscriptions infrastructure

# Instance 4
bv claim bd-vwb  # Notifications infrastructure
```

**Wait for all 4 to complete before Stage 4**

### Phase B: Documentation (Independent)
```bash
bv claim bd-1yf  # Can run anytime
```

### Phase C: Vertical Slice (Sequential)
```bash
# Must run in order:
bv claim bd-2x8  # 6.1: AR emits collection
# ↓ wait for completion
bv claim bd-1h0  # 6.2: Payments consumes
# ↓ wait for completion
bv claim bd-2nh  # 6.3: AR applies payment

# These can run in parallel once their deps are met:
bv claim bd-3fm  # 6.4: Notifications (needs bd-vwb)
bv claim bd-346  # 6.5: Subscriptions (needs bd-2go)
```

### Phase D: Validation (After A)
```bash
bv claim bd-2yq  # Add validation
```

### Phase E: E2E Proof (Final)
```bash
bv claim bd-mv0  # After all vertical slice beads
```

---

## 📊 Dependency Tree

```
bd-mv0: 6.6: End-to-end proof test
  ├── bd-1h0: 6.2: Payments consumes and emits result
    ├── bd-2hs: 5.1b: Payments module outbox/inbox infrastructure
    └── bd-2x8: 6.1: AR emits collection command
      └── bd-2b4: 5.1a: AR module outbox/inbox infrastructure
  ├── bd-2nh: 6.3: AR applies payment
    ├── bd-1h0: 6.2: Payments consumes and emits result (see above)
    └── bd-2b4: 5.1a: AR module outbox/inbox infrastructure
  ├── bd-2x8: 6.1: AR emits collection command (see above)
  ├── bd-346: 6.5: Subscriptions bill-run integration
    └── bd-2go: 5.1c: Subscriptions module outbox/inbox infrastructure
  └── bd-3fm: 6.4: Notifications react to events
    └── bd-vwb: 5.1d: Notifications module outbox/inbox infrastructure
```

---

## 🔥 Critical Path

Longest dependency chain:
```
5.1a (AR infra) → 6.1 (AR emit) → 6.2 (Payments) → 6.3 (AR apply) → 6.6 (E2E)
```

**Total sequential beads on critical path:** 5 beads

---

## ✅ Execution Rules

**MANDATORY for all instances:**
- Additive changes only
- No refactors
- No cross-module imports
- Each module uses its own DB
- EventBus only via `Arc<dyn EventBus>`
- No direct NATS calls in modules
- Compile after each bead
- Prefix commits with bead ID: `[bd-xxx] Your message`

**Gate Checks:**
- Stage 1 → All modules `cargo check` clean
- Stage 1 → All modules run with `BUS_TYPE=inmemory`
- Stage 4 → Each bead gates the next (sequential)
- Stage 5 → All vertical slice complete

---

## 🎬 Recommended Start

**For single agent:**
```bash
bv claim bd-2b4  # Start with AR infrastructure
```

**For parallel team (4+ agents):**
```bash
# Each agent claims one infrastructure bead:
# Agent A: bv claim bd-2b4
# Agent B: bv claim bd-2hs
# Agent C: bv claim bd-2go
# Agent D: bv claim bd-vwb
```

---

## 📝 Notes

- bd-1yf (documentation) can be done anytime, no blocking
- bd-2yq (validation) should be done after Stage 1 but before production
- Subscriptions (6.5) runs parallel to AR flow but required for E2E
- All beads are tagged for easy filtering
- Use `br dep tree <bead-id>` to verify dependencies
- Use `bv recommend` to get next available bead

---

**Next Action:** Claim a Stage 1 bead or run `bv recommend` for smart suggestion.
