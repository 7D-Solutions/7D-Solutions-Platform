# Review: HTTP Smoke Test Plan — Full Route Coverage

**Date:** 2026-03-07
**Reviewer:** Automated Platform Review Agent
**Document:** `docs/reviews/inbox/review-http-smoke-test-plan-20260307.md`

---

## Summary Assessment: NEEDS WORK

The plan is solid in its overall structure and correctly identifies the severity of the HTTP coverage gap (6% → target 100%). The bead decomposition is sensible and the two critical chains are the right ones to prioritize. However, there are several missing risk areas, underspecified test patterns, and ordering issues that should be addressed before execution begins.

---

## Specific Findings

### 1. Test Pattern Gaps (Lines 17–23)

**Finding:** The five-point test pattern is a good baseline but omits several critical verification categories for an aerospace/defense customer.

**Missing verifications:**
- **Input validation / malformed request bodies:** Every route should be hit with at least one malformed payload (missing required fields, wrong types, oversized strings) to confirm the API returns 400/422 and not a 500 or stack trace.
- **Idempotency:** POST endpoints that create financial records (invoices, payments, journal entries) should be tested for idempotent behavior if the API supports idempotency keys.
- **Rate limiting / abuse:** At minimum, confirm that unauthenticated hammering returns 429 rather than causing resource exhaustion.
- **Content-Type enforcement:** Verify that sending `text/plain` or `application/xml` to JSON-only endpoints returns a clean 415, not a panic.
- **Response schema validation:** "Valid JSON response" (line 21) is too loose. Each route's response should be checked against its expected schema (required fields present, correct types). A smoke test that only checks `status == 200 && body.is_json()` will miss silent serialization regressions.

### 2. Auth Testing Is Underspecified (Line 22)

**Finding:** "Unauthenticated request → 401/403" is necessary but insufficient.

**Recommended additions:**
- **Cross-tenant isolation:** Authenticated user from Tenant A must get 403/404 when accessing Tenant B's resources. This is the single highest-risk gap for a multi-tenant SaaS platform serving defense customers.
- **Role-based access:** Test that a read-only user cannot hit write/delete endpoints. At minimum, one RBAC negative test per module.
- **Expired/revoked tokens:** Confirm that expired JWTs and revoked sessions return 401, not stale cached data.

### 3. Missing Cross-Module Test Sequences (Lines 114–118)

**Finding:** The two named chains are correct but incomplete. Several critical cross-module flows are absent.

**Recommended additions:**
- **Procurement chain:** Party (Vendor) → AP Purchase Order → Inventory Receipt → AP Invoice → Payment → GL
- **Maintenance chain:** Fixed Assets → Maintenance Plans → Maintenance Work Orders → Inventory (spare parts issue) → Timekeeping (labor)
- **Payroll/labor chain:** Workforce-Competence → Timekeeping entries → AP (if contractor) or GL (if payroll accrual)
- **Document lifecycle:** Any entity creation → Doc-Mgmt attachment → PDF Editor rendering → Shipping-Receiving (packing slip)
- **Subscription/billing chain:** Subscriptions → AR (bill-usage) → Payments → GL

### 4. State Machine Dependencies Not Enumerated (Lines 8–9)

**Finding:** The plan asks for identification of ordering dependencies but does not enumerate them. These must be explicit before beads begin, or tests will be flaky.

**Known state machine dependencies that require ordered test steps:**
- AR Invoice: Draft → Finalized → Paid (cannot allocate payment to draft invoice)
- Production Work Order: Created → Released → In-Progress → Completed (operations and component issues only valid in certain states)
- GL Period: Open → Closing → Closed → Reopened (journal entry posting blocked in closed period)
- Maintenance Work Order: Requested → Approved → In-Progress → Completed
- Quality Inspection: Pending → In-Progress → Pass/Fail (cannot ship a failed inspection)
- Shipping: Requires completed QI for lot-traced items
- Workflow: Approval chains must complete before entity state transitions

**Recommendation:** Each bead's description should include the required state progression so the implementer doesn't have to reverse-engineer it.

### 5. Bead Sizing Concerns

**Finding:** Some "single-bead" modules are quite large.

- **AP at 20 untested routes** is comparable to Production (26 routes, split into 2 beads). AP has complex state machines (PO → Receipt → Invoice → Payment) and should be split into at least 2 beads: AP PO/Receipt lifecycle and AP Invoice/Payment lifecycle.
- **Consolidation at 21 untested routes** involves multi-entity financial consolidation with elimination entries — this is inherently complex and should also be split.
- **Treasury at 19 untested routes** includes bank reconciliation, cash forecasting, and FX — recommend splitting into Treasury Core and Treasury Recon/FX.

### 6. No Error/Failure Scenario Coverage

**Finding:** The plan focuses entirely on happy-path and auth. Smoke tests should also cover:

- **Conflict scenarios:** Concurrent updates to the same entity (optimistic locking / 409 responses)
- **Not-found handling:** GET/PUT/DELETE on non-existent UUIDs should return 404, not 500
- **Soft-delete behavior:** Accessing soft-deleted entities should return 404 or 410, not the deleted record
- **Pagination edge cases:** Empty result sets, page beyond total, negative page numbers

### 7. Docker Environment Specification (Line 24)

**Finding:** "All tests run against live Docker containers" lacks specifics.

**Questions to resolve:**
- Is there a single shared Docker Compose, or per-module? Shared environments risk cross-test pollution.
- Database seeding: Is each test responsible for its own seed data, or is there a shared fixture? Per-test seeding (with unique tenant_id) is safer but slower.
- Are external dependencies (email, payment gateways, PDF renderer) stubbed or live? Smoke tests should hit real stubs, not mocks, to catch serialization issues.

### 8. Missing Module: Audit Trail

**Finding:** There is no mention of audit logging verification. For aerospace/defense compliance (ITAR, DFARS, AS9100), every mutating API call should produce an audit record. Recommend adding a cross-cutting verification: after each POST/PUT/DELETE, query the audit trail endpoint and confirm an entry was created.

---

## Risk Assessment for First Customer (Aerospace/Defense)

| Risk | Severity | Mitigation |
|------|----------|------------|
| Cross-tenant data leakage not tested | **CRITICAL** | Add tenant isolation tests to every module's bead |
| No audit trail verification | **HIGH** | Add audit log assertions as cross-cutting concern |
| Auth testing limited to presence/absence | **HIGH** | Add RBAC negative tests and expired-token tests |
| Input validation not tested | **HIGH** | Add malformed-payload test per route |
| State machine ordering undocumented | **MEDIUM** | Document required state progressions per bead |
| AP/Consolidation/Treasury beads too large | **MEDIUM** | Split into smaller beads to reduce risk of incomplete work |
| No conflict/concurrency testing | **MEDIUM** | Add at least one optimistic-locking test per write-heavy module |
| Docker environment underspecified | **LOW** | Document Compose topology and stub strategy before starting |

---

## Recommended Additions Summary

1. **Expand the test pattern** (lines 17–23) to include input validation, schema validation, content-type enforcement, and idempotency checks.
2. **Add cross-tenant isolation tests** as a mandatory verification for every module — this is the #1 risk for a defense customer.
3. **Add RBAC negative tests** (at least one per module).
4. **Enumerate state machine progressions** in each bead description before work begins.
5. **Split AP, Consolidation, and Treasury** into 2 beads each (total becomes ~41 beads).
6. **Add 3–4 more cross-module chains** (procurement, maintenance, payroll, document lifecycle).
7. **Add audit trail verification** as a cross-cutting smoke test concern.
8. **Specify Docker environment** details: Compose topology, seed strategy, stub vs. live dependencies.
9. **Add error-path smoke tests:** 404 on missing resources, 409 on conflicts, 400/422 on bad input, 415 on wrong content-type.

---

## Conclusion

The plan correctly identifies the problem (94% of routes untested at HTTP level) and proposes a reasonable decomposition. The main gaps are around security testing depth (tenant isolation, RBAC, input validation), compliance requirements (audit trail), and execution specifics (state machine ordering, Docker environment). Addressing these before the first bead starts will prevent significant rework and reduce risk for the aerospace/defense customer launch.
