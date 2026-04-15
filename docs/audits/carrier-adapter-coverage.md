# Carrier Adapter Test Coverage Audit

**Bead:** bd-agk66  
**Date:** 2026-04-14  
**Scope:** modules/shipping-receiving — FedEx, UPS, and USPS real-sandbox adapter test coverage

---

## FedEx coverage

Test file: `modules/shipping-receiving/tests/fedex_carrier.rs` (introduced bd-ttdso)

All sandbox tests are `#[ignore]` — they call the real FedEx Developer sandbox API and require
`FEDEX_CLIENT_ID`, `FEDEX_CLIENT_SECRET`, and `FEDEX_ACCOUNT_NUMBER` environment variables.
They run in the `carrier-integration` CI job.

| Test name | What it covers |
|-----------|---------------|
| `fedex_carrier_oauth_token_acquisition_succeeds` | OAuth2 client-credentials flow; token acquired before first rate call |
| `fedex_carrier_get_rates_returns_quotes_for_domestic_package` | Rate quote for 10 lb 12×12×12 domestic package; validates carrier_code, service_level, charge, currency; soft-asserts Ground and Express service types |
| `fedex_carrier_create_label_returns_tracking_number` | Label generation; validates 12–34 digit tracking number and PDF label format |
| `fedex_carrier_track_created_label_returns_valid_status` | Full label→track round-trip; validates tracking status and event list |
| `fedex_carrier_registry_resolves_fedex_provider` | **Always runs (non-ignored).** Registry wiring: `get_provider("fedex")` returns a provider with `carrier_code() == "fedex"` |

**Coverage summary:** OAuth, rate quote, label generation, tracking — all four primary operations covered.

---

## UPS coverage

Test file: `modules/shipping-receiving/tests/ups_carrier.rs` (introduced bd-2xl19)

All sandbox tests are `#[ignore]` — they call the real UPS CIE sandbox API and require
`UPS_CLIENT_ID`, `UPS_CLIENT_SECRET`, and `UPS_ACCOUNT_NUMBER` environment variables.

| Test name | What it covers |
|-----------|---------------|
| `ups_carrier_oauth_token_acquisition_succeeds` | OAuth2 token acquisition; also validates token cache (second call reuses token) |
| `ups_carrier_get_rates_returns_quotes_for_domestic_package` | Rate quote for 10 lb 12×12×12 domestic package; validates carrier_code, service_level, charge, currency |
| `ups_carrier_create_label_returns_tracking_number_and_label_image` | Label generation; validates `1Z`-prefixed tracking number and non-empty label bytes |
| `ups_carrier_create_label_and_track_returns_valid_status` | Full label→track round-trip; validates tracking status |
| `ups_carrier_registry_resolves_ups_provider` | **Always runs (non-ignored).** Registry wiring: `get_provider("ups")` returns a provider with `carrier_code() == "ups"` |

**Coverage summary:** OAuth (including token cache), rate quote, label generation, tracking — all four primary operations covered.

---

## USPS coverage

Test file: `modules/shipping-receiving/tests/usps_carrier.rs` (introduced bd-1z8bl)

All sandbox tests are `#[ignore]` — they call the real USPS Web Tools staging API
(`stg-production.shippingapis.com`). Require `USPS_USER_ID` environment variable.
Tests skip cleanly (no panic) when `USPS_USER_ID` is absent.

| Test name | What it covers |
|-----------|---------------|
| `usps_carrier_get_rates_returns_quotes_for_domestic_package` | Rate quote for 10 lb 12×12×12 domestic package; validates carrier_code, service_level, charge, currency |
| `usps_carrier_create_label_returns_tracking_and_label_bytes` | Label generation; validates non-empty tracking number and PDF label bytes |
| `usps_carrier_track_known_number_returns_valid_status` | Tracking via USPS-provided known test tracking number `9400110200882774868522`; validates status |
| `usps_carrier_registry_resolves_usps_provider` | **Always runs (non-ignored).** Registry wiring: `get_provider("usps")` returns a provider with `carrier_code() == "usps"` |

**Coverage summary:** Rate quote, label generation, tracking — three primary operations covered. USPS does not use OAuth2 (User ID header auth), so no OAuth test is applicable.

---

## Gaps

### Carrier-level gaps

All three adapters have sandbox coverage for the core happy-path operations. The following gaps exist:

#### Error path testing (all three adapters)

No tests cover API error responses:
- Invalid credentials → expect authentication error, not panic
- Rate request for unsupported service zone → expect empty rates or error, not panic
- Label creation with bad address (USPS returns an XML `<Error>` block; FedEx/UPS return HTTP 4xx) → expect clear error propagation
- Track for unknown/invalid tracking number → expect explicit error, not empty events

**Missing test paths (apply to each carrier file):**
- `fedex_carrier_invalid_credentials_returns_auth_error`
- `fedex_carrier_track_invalid_number_returns_error`
- `ups_carrier_invalid_credentials_returns_auth_error`
- `ups_carrier_track_invalid_number_returns_error`
- `usps_carrier_invalid_user_id_returns_auth_error`
- `usps_carrier_track_invalid_number_returns_error`

#### USPS label tracking round-trip

USPS label test (`usps_carrier_create_label_returns_tracking_and_label_bytes`) does not
chain into a track call using the created label's tracking number. The tracking test uses
a hard-coded test number instead. The gap: a label created via the sandbox may produce a
tracking number that is not yet visible in the USPS tracking sandbox — this is a known
USPS sandbox limitation, not a test design flaw.

#### FedEx/UPS: large-package / dimensional weight

No test covers packages where dimensional weight exceeds actual weight. This is a common
billing source of truth gap — the carrier charges dimensional weight, but no test verifies
the adapter returns the dimensional-weight-adjusted charge.

#### International rates

All rate tests use domestic US ZIP codes. International rate requests (to non-US
destinations) are untested for all three adapters.

---

## Recommended child beads

These are candidates for follow-on beads. Do not create them here — list them as input to
the next planning cycle.

| Title | Scope |
|-------|-------|
| `test: fedex carrier error-path coverage` | Add `#[ignore]` sandbox tests: invalid credentials → auth error; label with bad address → error; track invalid number → error. File: `modules/shipping-receiving/tests/fedex_carrier.rs`. |
| `test: ups carrier error-path coverage` | Same matrix for UPS. File: `modules/shipping-receiving/tests/ups_carrier.rs`. |
| `test: usps carrier error-path coverage` | Same matrix for USPS (invalid user_id, bad address, invalid tracking). File: `modules/shipping-receiving/tests/usps_carrier.rs`. |
| `test: fedex/ups dimensional-weight rate coverage` | Add sandbox test with package where DIM weight > actual weight; assert returned charge reflects DIM weight. |
| `test: carrier international rate coverage` | Add rate tests with a Canadian or EU destination address for each adapter that supports international shipping. |
