# Party Master Module — Vision & Roadmap

**Version**: 0.1.0
**Last Updated**: 2026-02-25
**Status**: Active

---

## 1. Mission

Party Master is the **canonical identity registry** for all external entities in the platform — customers, vendors, carriers, employees, and any other organization or individual the business interacts with. Every module that needs to reference an external entity does so via a `party_id` from this module. Party Master prevents identity fragmentation across the ERP.

### Non-Goals

Party does **NOT**:
- Own financial relationships (AR owns customer billing, AP owns vendor payables)
- Own employment records (Timekeeping owns employee time data)
- Own carrier logistics (Shipping-Receiving owns shipment tracking)
- Store module-specific attributes (each module stores its own context against a party_id)

---

## 2. Domain Authority

| Domain Entity | Party Authority |
|---|---|
| **Parties** | Canonical identity: name, type (org/person), status (active/inactive), tax IDs |
| **External Refs** | Cross-system references (ERP IDs, tax IDs, government IDs) |
| **Contacts** | Contact persons linked to parties (name, email, phone, role) |
| **Addresses** | Physical and mailing addresses linked to parties |

---

## 3. Data Ownership

| Table | Purpose |
|---|---|
| `party_parties` | Core identity records with type and status |
| `party_companies` | Company extension (1:1) |
| `party_individuals` | Individual extension (1:1) |
| `party_external_refs` | External system cross-references |
| `party_contacts` | Contact persons per party |
| `party_addresses` | Typed addresses per party |
| `party_idempotency_keys` | HTTP idempotency keys |
| `party_outbox` | Module outbox for NATS |
| `party_processed_events` | Consumer idempotency |

---

## 4. Events

**Produces:**
- `party.created` — new party registered
- `party.updated` — party record modified
- `party.deactivated` — party deactivated

**Consumes:**
- None (Party is a reference system — other modules read via HTTP API)

---

## 5. Key Invariants

1. Party IDs are globally unique per tenant
2. External refs are unique per (tenant, ref_type, ref_value)
3. Deactivated parties cannot be re-referenced in new transactions
4. Tenant isolation on every table and query
5. No module-specific business logic in Party — it is pure identity

---

## 6. Integration Map

- **AP** → vendors reference `party_id`
- **AR** → customers reference `party_id`
- **Shipping-Receiving** → carriers reference `carrier_party_id`
- **TTP** → service parties reference `party_id`
- **Timekeeping** → employees may reference `party_id` (future)

---

## 7. Roadmap

### v0.1.0 (current)
- Party CRUD (create, update, deactivate)
- Contact management per party
- Address management per party
- External reference linking
- Event emission for lifecycle changes

### v1.0.0 (proven)
- Party merge/deduplication workflow
- Hierarchical party relationships (parent/child orgs)
- Party classification tags
- Bulk import/export
- Address validation integration
