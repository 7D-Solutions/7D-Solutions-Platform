# Integrations Module — Vision & Roadmap

**Version**: 0.1.0
**Last Updated**: 2026-02-25
**Status**: Active

---

## 1. Mission

Integrations is the **external connectivity layer** — it manages webhook ingestion, external system connectors, and cross-system entity reference mapping. When the platform needs to receive data from or link to external systems, Integrations provides the plumbing.

### Non-Goals

Integrations does **NOT**:
- Own any business domain data (financial, inventory, etc.)
- Execute business logic (it routes; target modules process)
- Own payment processor webhooks (Payments handles its own)
- Replace module-specific API clients

---

## 2. Domain Authority

| Domain Entity | Integrations Authority |
|---|---|
| **Connector Configs** | External system connection settings and credentials |
| **External Refs** | Cross-system entity reference mapping (e.g., "Xero invoice X = AR invoice Y") |
| **Webhook Endpoints** | Registered webhook receiver configurations |
| **Webhook Ingest** | Inbound webhook event log and routing |

---

## 3. Data Ownership

| Table | Purpose |
|---|---|
| `connector_configs` | External system connection settings |
| `external_refs` | Cross-system entity mappings |
| `webhook_endpoints` | Webhook receiver registrations |
| `webhook_ingest` | Inbound webhook event log |
| `events_outbox` | Module outbox for NATS |
| `processed_events` | Consumer idempotency |

---

## 4. Events

**Produces:**
- `external_ref.created` — new cross-system reference
- `external_ref.updated` — reference mapping updated
- `external_ref.deleted` — reference mapping removed
- `webhook.received` — inbound webhook received
- `webhook.routed` — webhook routed to target module

**Consumes:**
- Various (routes inbound webhooks to target modules based on config)

---

## 5. Key Invariants

1. External refs are unique per (tenant, source_system, ref_type, external_id)
2. Webhook signatures must be verified before processing
3. All inbound webhooks logged for audit regardless of processing outcome
4. Connector credentials encrypted at rest
5. Tenant isolation on every table and query

---

## 6. Integration Map

- **All modules** → Integrations provides external ref mapping for any module entity
- **External systems** → Webhooks routed to target modules via config
- **Reporting** → future: sync status dashboards

---

## 7. Roadmap

### v0.1.0 (current)
- Connector config management
- External reference CRUD
- Webhook endpoint registration
- Webhook ingestion with signature verification
- Webhook routing to target modules
- Event emission for ref and webhook lifecycle

### v1.0.0 (proven)
- OAuth2 connector authentication flow
- Scheduled sync jobs (pull-based integration)
- Transformation/mapping rules for data normalization
- Integration health monitoring and alerting
- Rate limiting and backpressure for high-volume connectors
