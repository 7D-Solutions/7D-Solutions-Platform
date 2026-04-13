# Audit Standard

Every state-changing operation in the 7D Solutions Platform must write an audit record
inside the **same database transaction** as the mutation. If the mutation rolls back,
the audit record rolls back with it. There are no orphaned audit rows and no silent
mutations.

## The Rule

> For every write endpoint, call `AuditWriter::write_in_tx(&mut tx, req)` before
> `tx.commit()`. The audit write is the last step before commit.

## Mutation Classes

| `MutationClass` | When to use |
|---|---|
| `Create` | A new entity is inserted (bill, invoice, journal entry, work order, etc.) |
| `Update` | Fields on an existing entity are changed in place |
| `StateTransition` | A lifecycle state machine transition (approve, release, close, finalize, void) |
| `Reversal` | An entry is voided or reversed to negate a prior financial effect |
| `Delete` | Hard delete — rare, only used for non-financial reference data |

## Covered Mutations per Module

### AP (Accounts Payable)
| Operation | Action string | MutationClass |
|---|---|---|
| Create vendor bill | `CreateVendorBill` | `Create` |
| Approve vendor bill | `ApproveBill` | `StateTransition` |
| Void vendor bill | `VoidBill` | `Reversal` |

### AR (Accounts Receivable)
| Operation | Action string | MutationClass |
|---|---|---|
| Create invoice | `CreateInvoice` | `Create` |
| Finalize invoice | `FinalizeInvoice` | `StateTransition` |

### GL (General Ledger)
| Operation | Action string | MutationClass |
|---|---|---|
| Post journal entry | `PostJournalEntry` | `Create` |
| Close accounting period | `ClosePeriod` | `StateTransition` |

### Production
| Operation | Action string | MutationClass |
|---|---|---|
| Create work order | `CreateWorkOrder` | `Create` |
| Release work order | `ReleaseWorkOrder` | `StateTransition` |
| Close work order | `CloseWorkOrder` | `StateTransition` |
| Request component issue | `RequestComponentIssue` | `Create` |
| Request FG receipt | `RequestFgReceipt` | `Create` |

### Inventory
| Operation | Action string | MutationClass |
|---|---|---|
| Stock adjustments | see Inventory module | `Create` / `Reversal` |

### Control Plane
| Operation | Action string | MutationClass |
|---|---|---|
| Tenant lifecycle events | see control-plane module | `StateTransition` |

## Actor Convention

Service-layer operations that lack a user identity use `Uuid::nil()` as `actor_id`
and `"system"` as `actor_type`. HTTP handlers that have a verified JWT claim should
pass the `user_id` from `VerifiedClaims` and `"user"` as `actor_type`.

## Implementation Pattern

```rust
use platform_audit::schema::{MutationClass, WriteAuditRequest};
use platform_audit::writer::AuditWriter;

// ... inside a transaction block, after the mutation ...

let audit_req = WriteAuditRequest::new(
    Uuid::nil(),           // actor_id  (use real user_id when available)
    "system".to_string(),  // actor_type
    "CreateFoo".to_string(), // action
    MutationClass::Create,
    "Foo".to_string(),     // entity_type
    entity_id.to_string(), // entity_id
);
AuditWriter::write_in_tx(&mut tx, audit_req).await
    .map_err(|e| match e {
        platform_audit::writer::AuditWriterError::Database(db) => MyError::Database(db),
        platform_audit::writer::AuditWriterError::InvalidRequest(msg) => {
            MyError::Database(sqlx::Error::Protocol(msg))
        }
    })?;

tx.commit().await?;
```

## Cargo Dependency

Add to each module's `Cargo.toml`:

```toml
platform-audit = { package = "audit", path = "../../platform/audit" }
```

The alias (`platform-audit`) ensures all Rust usage appears as `platform_audit::` in
source, satisfying the density check: `grep -rn 'platform_audit' modules/ --include='*.rs'`
must return >= 30 matches.

## Database Migration

Each module requires its own `audit_events` table and `mutation_class` enum since each
module has its own PostgreSQL database. Copy the migration from:

```
platform/audit/db/migrations/20260216000001_create_audit_log.sql
```

into each module's `db/migrations/` directory with an appropriate timestamp prefix.

## Verification

Each module has an `audit_oracle` integration test in `tests/audit_oracle.rs` that:
1. Calls the service function against a real database
2. Queries `audit_events` for the entity
3. Asserts exactly 1 record with the expected `mutation_class`, `entity_id`, and `actor_id`

Run all oracles:
```bash
./scripts/cargo-slot.sh test -p ap-rs audit_oracle -- --nocapture
./scripts/cargo-slot.sh test -p ar-rs audit_oracle -- --nocapture
./scripts/cargo-slot.sh test -p gl audit_oracle -- --nocapture
./scripts/cargo-slot.sh test -p production-rs audit_oracle -- --nocapture
```
