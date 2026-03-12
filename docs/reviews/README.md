# Claude Desktop Review Drop

## Paths
- **Inbox (you poll this):** `/Users/james/Projects/7D-Solutions Platform/docs/reviews/inbox/`
- **Outbox (you write here):** `/Users/james/Projects/7D-Solutions Platform/docs/reviews/outbox/`

## Instructions for Claude Desktop

1. Poll `docs/reviews/inbox/` for new `.md` files
2. For each file found:
   - Read it completely
   - Analyze the plan/document for: completeness, risks, missing test cases, logical errors
   - Write your review to `docs/reviews/outbox/` with the same filename prefixed with `reviewed-`
   - Example: `inbox/review-http-smoke-test-plan-20260307.md` -> `outbox/reviewed-http-smoke-test-plan-20260307.md`
3. Your review should include:
   - Summary assessment (PASS / NEEDS WORK / CRITICAL GAPS)
   - Specific findings with line references
   - Recommended additions or changes
   - Risk assessment for first customer (aerospace/defense)

## Current documents awaiting review
- `inbox/review-http-smoke-test-plan-20260307.md` — Full HTTP smoke test plan for 443 API routes across 24 modules
