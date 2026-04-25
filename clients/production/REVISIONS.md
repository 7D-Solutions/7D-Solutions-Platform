# platform-client-production — Revision History

## 0.1.1 — 2026-04-25 (bd-xfs7e)

**Fix:** `WorkOrder.bom_revision_id` changed from `uuid::Uuid` to `Option<uuid::Uuid>` to match server `WorkOrderResponse` schema. The field is nullable in the database; the client struct was incorrectly non-optional causing deserialization failures for work orders without a BOM revision (25% error rate reported by Huber Power).

Updated `openapi.json` to mark the field nullable (`type: ["string", "null"]`, removed from `required`). Client regenerated via `client-codegen`.

## 0.1.0 — initial release
