# Platform Frontend — Language & Formatter Standards

> **Who reads this:** Any agent writing labels, display values, error messages, or formatting dates and numbers.
> **What it covers:** What language appears in the UI (plain English rules) and exactly how dates, currencies, and numbers are formatted.
> **Rule:** Every display value follows these rules. No exceptions for "it's just a label."

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Extracted from PLATFORM-FRONTEND-STANDARDS.md rev 1.8. Language standards (staff-facing and customer-facing rules), formatter standards (date/currency/numeric rules and implementation pattern). Decision Log populated from master. |

---

## Language Standards

Users should never see internal system terminology. Every label, status, action, and error message must be plain English.

### Rules That Apply to All Apps

- Never show database column names in the UI (`tenant_id`, `app_id`, `ar_customer_id` are never visible to users)
- Never show system codes as display values (`DELINQUENT`, `IN_CALIBRATION`, `PROVISIONING_STATE` stay in the backend)
- Boolean fields: display as "Yes" / "No" — never `true` / `false`
- Dates: always formatted per the Formatter Standards below — never raw ISO strings
- Currency: always formatted per the Formatter Standards below — never raw numbers
- Error messages: state what happened AND what the user should do next. "Something went wrong" is not acceptable.
- Status is always color + text (via `StatusBadge`) — never code only

### Rules for Staff-Facing Apps (Admin Consoles)

- Use precise language — staff can handle exact descriptions. "This will terminate the tenant and cancel all active subscriptions immediately" is better than "Are you sure?"
- Confirmation dialogs name what will happen specifically: "Terminate Acme Corp?" not "Are you sure?"
- Action buttons say what they do: "Suspend" not "Change Status"
- Field labels match what the field contains: "Connection ID" not "App ID" (see each app's language translation table in its vision doc)

### Rules for Customer-Facing Apps

- Conversational tone throughout — "We couldn't load this" not "Error fetching resource"
- Dates in conversational format: "Tuesday, March 4th at 8:14am" — never any ISO format
- Errors must include a contact method: "We couldn't load this — try again or call 555-0100" not "Error 503"
- No confirmation dialogs for non-destructive actions the user deliberately initiated (one tap to pay, one tap to confirm a delivery)
- Maximum 3 data columns visible at once in any table — prefer cards over tables
- No internal terminology visible anywhere — not even in tooltips

### Language Translation Tables

Each app maintains a table in its vision document mapping internal system terms to staff-facing or customer-facing labels. This table is authoritative — agents use it when labeling any UI element.

See: `docs/frontend/TCP-UI-VISION.md` → Language Standards section for TCP UI translations.

---

## Formatter Standards

Every app implements a local `infrastructure/utils/formatters.ts` following these rules exactly. No app invents its own date or currency format. Consistent formatting across the platform builds trust — inconsistency looks broken.

Reference implementation: `docs/reference/fireproof/src/infrastructure/utils/dateFormatters.ts` and `docs/reference/fireproof/src/infrastructure/utils/formatters.ts`

### Date Formatting Rules

| Context | Format | Example |
|---------|--------|---------|
| Within last 7 days | Relative | "Just now", "2 minutes ago", "1 hour ago", "Yesterday at 3pm", "3 days ago" |
| Beyond 7 days | Short date | "Feb 20, 2026" |
| Audit / activity events | Short date + time, always | "Feb 20, 2026 at 10:37am" |
| Long dates (e.g., "February 20th, 2026") | Never | — |
| Raw ISO strings (e.g., "2026-02-20T10:37:00Z") | Never visible to users | — |

**Relative time precision:**
- Under 60 seconds: "Just now"
- Under 60 minutes: "X minutes ago"
- Under 24 hours: "X hours ago"
- Yesterday: "Yesterday at [time]" (e.g., "Yesterday at 3pm")
- 2–7 days ago: "X days ago"
- Beyond 7 days: fall through to short date format

**Why audit events always include time:** An audit entry without a time is useless for debugging. "Feb 20, 2026" tells you nothing about when in the day something happened.

### Currency Formatting Rules

- Always use `Intl.NumberFormat` — never format currency manually with string manipulation
- Always include the currency symbol
- Currency code comes from the data — never hardcoded to USD or any other currency
- Standard format:

```typescript
const formatCurrency = (amount: number, currency: string) =>
  new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency,
    minimumFractionDigits: 2,
    maximumFractionDigits: 2
  }).format(amount);

// Output: "$1,234.56"
```

- For multi-currency display contexts (e.g., an invoice showing a foreign currency), show both symbol and code: "$1,234.56 USD"

### Numeric Formatting Rules

| Type | Format | Example |
|------|--------|---------|
| Percentages | One decimal place | "12.3%" (never "0.123" or "12.3456%") |
| Large integers | Comma-separated | "1,234,567" (use `Intl.NumberFormat`) |
| Financial decimals | Two decimal places | "1,234.56" |
| Seat counts, item counts | Integer, no decimals | "42 seats" |

### Implementation Pattern

Each app creates `infrastructure/utils/formatters.ts` and exports named formatters:

```typescript
// infrastructure/utils/formatters.ts

export const formatDate = (date: Date | string): string => { /* relative/short date logic */ };
export const formatDateTime = (date: Date | string): string => { /* always date + time */ };
export const formatCurrency = (amount: number, currency: string): string => { /* Intl.NumberFormat */ };
export const formatPercent = (value: number): string => { /* one decimal */ };
export const formatNumber = (value: number): string => { /* comma-separated */ };
```

**Rule:** Import from `formatters.ts` everywhere. Never call `Intl.NumberFormat` or `new Date().toLocaleDateString()` directly in a component.

---

## Open Questions

Do not create beads until these are resolved.

| # | Question | Status |
|---|----------|--------|
| — | No open questions at this time. | — |

---

## Decision Log

Decisions specific to language and formatting. Do not re-open without an explicit user directive.

| Date | Decision | Rationale (includes what was NOT chosen) | Decided By |
|------|----------|------------------------------------------|-----------|
| 2026-02-20 | Platform standards are product-agnostic — all app-specific content lives in each app's vision doc | Platform doc must be usable by any future app team without mental search-and-replace. Rejected: including app-specific examples inline (confuses future app teams). | User |
| 2026-02-20 | Date and currency formatting rules standardized — each app implements local formatters.ts following the rules | "Feb 20" vs "February 20th" vs "02/20" across apps looks broken. Consistent format builds trust. Rejected: each app formats dates independently (guaranteed drift). | Platform Orchestrator + TrashTech Orchestrator |
| 2026-02-20 | Audit and activity events always show date + time | Audit entries without time are useless for debugging. Rejected: relative-only format for audit events (loses precision when you need it most). | Platform Orchestrator |
| 2026-02-20 | Currency code comes from data — never hardcoded | Platform is multi-tenant and may serve tenants in different currency contexts. Rejected: hardcoding USD (breaks for international tenants). | Platform Orchestrator |

---

> See `docs/frontend/DOC-REVISION-STANDARDS.md` for the standards governing how this document is maintained.
