# Consolidation Module — Vision & Roadmap

**Version**: 0.1.0
**Last Updated**: 2026-02-25
**Status**: Active

---

## 1. Mission

Consolidation produces **multi-entity financial statements** by aggregating data from GL, AR, and AP across tenant entities, generating intercompany eliminations, and outputting consolidated views. It is a read-heavy aggregation module that never owns source financial data.

### Non-Goals

Consolidation does **NOT**:
- Own any financial source data (GL, AR, AP own their respective data)
- Post journal entries (elimination postings are internal to this module)
- Handle single-entity reporting (GL and Reporting modules cover that)

---

## 2. Domain Authority

| Domain Entity | Consolidation Authority |
|---|---|
| **Consolidation Configs** | Multi-entity grouping rules and ownership percentages |
| **Consolidation Caches** | Pre-computed consolidated balances |
| **Elimination Postings** | Intercompany elimination entries (internal to this module) |

---

## 3. Data Ownership

| Table | Purpose |
|---|---|
| `consolidation_config` | Entity grouping and ownership rules |
| `consolidation_caches` | Cached consolidated balance data |
| `elimination_postings` | Generated elimination entries |

---

## 4. Events

**Produces:** None (read-only aggregation module)

**Consumes:** None (reads GL/AR/AP data via HTTP APIs on demand)

---

## 5. Key Invariants

1. Consolidation never writes to GL, AR, or AP databases
2. Elimination postings are internal-only and do not affect source modules
3. Cache invalidation triggered on source data changes
4. Tenant isolation: consolidation configs are tenant-scoped

---

## 6. Integration Map

- **GL** → reads chart of accounts, journal entries, account balances via HTTP API
- **AR** → reads receivables data for consolidated AR aging
- **AP** → reads payables data for consolidated AP aging
- **Reporting** → future: consolidated data feeds into cross-entity dashboards

---

## 7. Roadmap

### v0.1.0 (current)
- Consolidation config management (entity groups, ownership %)
- Multi-entity balance aggregation
- Intercompany elimination generation
- Consolidated financial statement output
- Cache management for performance

### v1.0.0 (proven)
- Currency translation for foreign subsidiaries
- Minority interest calculations
- Consolidation audit trail
- Automated elimination rule detection
- Period-over-period comparison
