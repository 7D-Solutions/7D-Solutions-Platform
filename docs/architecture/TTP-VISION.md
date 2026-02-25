# TTP (TrashTech Pro) Module — Vision & Roadmap

**Version**: 2.0.1
**Last Updated**: 2026-02-25
**Status**: Proven (v2.x)

---

## 1. Mission

TTP is a **product-tier module** that implements the TrashTech Pro vertical — a waste management SaaS product built on the 7D platform. TTP owns usage metering, billing run execution, and invoice coordination for waste management tenants. It composes platform modules (AR, Party, Payments) into a cohesive product experience.

### Non-Goals

TTP does **NOT**:
- Own invoice lifecycle (AR owns invoices)
- Own payment processing (Payments module)
- Own party identity (Party Master)
- Own GL posting (GL module via AR)

---

## 2. Domain Authority

| Domain Entity | TTP Authority |
|---|---|
| **TTP Tenants/Service Configs** | Product-level tenant configuration and service setup |
| **Metering Events** | Usage/event ingestion for billing calculation |
| **Billing Runs** | Periodic billing execution batches |
| **Billing Run Items** | Per-customer line items in a billing run |
| **Billing Traces** | Audit trail for billing calculations (hash-verified) |

---

## 3. Data Ownership

| Table | Purpose |
|---|---|
| `ttp_tenants` / service configs | Product-level tenant settings |
| `metering_events` | Ingested usage events |
| `billing_runs` | Billing run headers with status |
| `billing_run_items` | Per-customer items (unique per run) |
| `billing_traces` | Hash-verified audit trail |
| `events_outbox` | Module outbox for NATS |
| `processed_events` | Consumer idempotency |

---

## 4. Events

**Produces:**
- `ttp.billing_run.created` — billing run initiated
- `ttp.billing_run.completed` — billing run finished successfully
- `ttp.billing_run.failed` — billing run failed
- `ttp.party.invoiced` — invoice created for a party

**Consumes:**
- None (TTP initiates billing; AR handles results)

---

## 5. Key Invariants

1. Billing run items are unique per (tenant, run, customer) — no duplicate charges
2. Billing traces are hash-verified for audit integrity
3. Metering events are idempotent on event_id
4. Tenant identity derived from VerifiedClaims JWT (v2.0.0 breaking change)
5. TTP calls AR HTTP API to create invoices — never writes AR tables

---

## 6. Integration Map

- **AR** → TTP calls AR HTTP API to create invoices from billing runs
- **Party** → TTP references party_id for customer identity
- **Payments** → indirect via AR (AR commands payment collection)

---

## 7. Roadmap

### v2.0.1 (current — proven)
- Usage metering event ingestion
- Billing run execution (calculate charges from metering)
- Billing trace audit with hash verification
- AR invoice creation coordination
- JWT-based tenant identity (v2.0.0 breaking change)

### v3.0.0 (next major)
- Route-based metering (GPS/route optimization data)
- Container tracking integration
- Service agreement management
- Customer portal self-service
- Multi-rate billing schedules

---

## 8. Decision Log

| # | Decision | Rationale |
|---|---|---|
| 1 | Product-tier module, not platform-tier | TTP is vertical-specific; platform modules are reusable across products |
| 2 | v2.0.0: tenant_id from JWT only | Removed tenant_id from request body/params; aligns with platform security standard |
| 3 | Calls AR API, does not write AR tables | Preserves module boundary; AR owns invoice truth |
