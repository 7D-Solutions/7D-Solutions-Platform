# QBO Smoke Test Results

**Date**: 2026-03-28 12:37:11 UTC
**Realm**: 9341456702925820
**Sandbox**: https://sandbox-quickbooks.api.intuit.com/v3

## Summary

| Metric | Count |
|--------|-------|
| Passed | 46 |
| Failed | 1 |
| Total  | 47 |

## Failures

- **CDC 31d boundary**: Expected 400, got 200 — QBO may allow >30d in sandbox

## Tests Run

1. Token refresh cycle (refresh, rotation, stale token rejection)
2. Entity reads: Customer, Invoice, Payment, Item, Vendor, Account, Estimate, PurchaseOrder, Bill
3. Shipping writeback: ShipDate, TrackingNum, ShipMethodRef via sparse update + re-read verify
4. SyncToken conflict: stale token produces 400 / code 5010
5. CDC: 1h, 24h, 29d lookback + 31d rejection + full payload verification
6. Pagination: COUNT, STARTPOSITION cross-check, past-end empty set
7. Error cases: non-existent entity, malformed query, bad token, missing fields, fake entity
8. Create operations: customer + invoice creation + query readback
9. Special characters: LIKE wildcards, empty result set, long query, ORDER BY
10. Concurrent burst: 20 simultaneous requests, rate limit detection
