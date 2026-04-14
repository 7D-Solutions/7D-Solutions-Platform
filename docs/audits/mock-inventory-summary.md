# Mock Inventory Summary

This audit found 40 mocked, stubbed, fake, placeholder, or todo-comment constructs across the repo.
The detailed CSV is at [docs/audits/mock-inventory.csv](/Users/james/Projects/7D-Solutions%20Platform/docs/audits/mock-inventory.csv).

## By Module

## e2e-tests
- `mock`: 3
- `fake`: 4
- `todo-comment`: 1
- Highest risk: tenant isolation spoof tests and the payments outbox TODO

## integrations
- `mock`: 3
- `stub`: 1
- Highest risk: QBO and eBay HTTP stubs

## inventory
- `placeholder`: 7
- `fake`: 1
- Highest risk: compile-only placeholder routes in `modules/inventory/src/http/*`

## notifications
- `stub`: 1
- `fake`: 1
- Highest risk: the stub email server and fake admin DB pool

## ttp
- `fake`: 2
- Highest risk: fake claims helpers in billing and service-agreement tests

## ar
- `fake`: 1
- `todo-comment`: 2
- Highest risk: lifecycle and finalization TODOs in production code

## payments
- `fake`: 1
- `todo-comment`: 1
- Highest risk: payment lifecycle TODO in production code

## shipping-receiving
- `stub`: 1
- Highest risk: `StubCarrierProvider` is the only registered carrier implementation

## customer-portal
- `mock`: 1
- Highest risk: the mock Doc Management service in the real E2E test

## ap
- `fake`: 1

## consolidation
- `fake`: 1

## fixed-assets
- `fake`: 1

## gl
- `fake`: 1

## reporting
- `fake`: 1

## subscriptions
- `fake`: 1

## timekeeping
- `fake`: 1

## treasury
- `fake`: 1

## Top 10 by Risk

| # | File:Line | Kind | Risk | Notes |
|---|---|---|---:|---|
| 1 | `modules/shipping-receiving/src/domain/carrier_providers/stub.rs:14` | `stub` | 25 | Production carrier stub returns canned rates labels and tracking instead of real carriers |
| 2 | `e2e-tests/tests/tenant_isolation_spoof_e2e.rs:82` | `fake` | 20 | Router boundary test uses a fake lazy pool instead of a real database |
| 3 | `e2e-tests/tests/tenant_isolation_spoof_e2e.rs:88` | `fake` | 20 | Second fake lazy pool path in the same tenant spoofing test |
| 4 | `e2e-tests/tests/tenant_isolation_spoof_e2e.rs:107` | `fake` | 20 | Third fake lazy pool path in the tenant spoofing test |
| 5 | `e2e-tests/tests/payments_outbox_atomicity_e2e.rs:13` | `todo-comment` | 20 | Test documents missing outbox emission for payments lifecycle |
| 6 | `e2e-tests/tests/bill_run_e2e.rs:271` | `mock` | 16 | In-process mock payment consumer synthesizes payment.succeeded |
| 7 | `e2e-tests/tests/bill_run_e2e.rs:348` | `mock` | 16 | In-process mock AR consumer applies synthetic payment events |
| 8 | `e2e-tests/tests/bill_run_e2e.rs:397` | `mock` | 12 | In-process mock notification consumer emits notification events |
| 9 | `modules/integrations/tests/qbo_outbound.rs:191` | `mock` | 16 | Local QBO invoice server simulates outbound API responses |
| 10 | `modules/integrations/tests/ebay_connector.rs:306` | `stub` | 16 | Stub server returns canned OAuth and fulfillment responses |

## Notes

- Fake or stubbed helpers are concentrated in test harnesses and admin-router smoke tests.
- The highest-risk items are the production carrier stub and the security-adjacent tenant isolation spoof harness.
- Compile-only inventory placeholders are all in `modules/inventory/src/http/*` and are low risk but still worth tracking.
