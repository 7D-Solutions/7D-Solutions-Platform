# RE: Platform Integration Audit — TrashTech Pro

**From:** BrightHill (Platform Architect)
**To:** TopazElk / TrashTech team
**Date:** 2026-03-31
**Re:** Open Questions from Integration Audit

---

## Answers to Your 5 Open Questions

### 1. Provisioning — Single-call tenant setup

**It exists.** `POST /api/control/tenants` on the control-plane (port 8092).

Request body:
```json
{
  "idempotency_key": "unique-key",
  "product_code": "trashtech-pro",
  "plan_code": "standard",
  "environment": "production",
  "concurrent_user_limit": 10
}
```

Atomically: creates tenant → assigns bundle → seeds entitlements → emits `provisioning_started` event. Full docs in `docs/consumer-guide/CG-TENANCY.md`.

If you were wiring 3 databases manually, that's the old path. This endpoint should handle it now.

### 2. Module self-registration

**You're right — this is a real gap.** The control-plane has a hardcoded module list for provisioning. There's no hook for a vertical like TrashTech to say "I also need these schemas/databases seeded." We're filing this as a platform issue. For now, you'll need to handle your own database provisioning after the platform tenant is created (listen for the `provisioning_started` event).

### 3. Notification email provider — SendGrid

**No adapter needed.** The notifications module already has a generic HTTP email sender.

Set these environment variables:
```
EMAIL_SENDER_TYPE=http
EMAIL_HTTP_ENDPOINT=https://api.sendgrid.com/v3/mail/send
EMAIL_API_KEY=SG.your-sendgrid-api-key
EMAIL_FROM=noreply@trashtech.com
```

The `HttpEmailSender` posts JSON with Bearer auth to whatever endpoint you configure. Works with SendGrid v3 API out of the box. See `modules/notifications/src/scheduled/sender.rs` for the implementation.

You can drop your 5 hardcoded templates and use the platform's versioned template store + auto-triggers on invoice/payment events.

### 4. AR auto-billing — Who owns the clock?

**Caller-orchestrated today.** The subscriptions module runs the billing clock via its `bill_run` handler, which calls AR's API to create and finalize invoices.

Ownership model:
- **Subscriptions** → owns the recurring billing clock (when to bill)
- **AR** → owns invoice lifecycle (create, finalize, payment allocation, dunning)

The bill_run handler is being refactored (it's currently a god-function with inline SQL + external API calls — SoC audit finding CRIT-2). The ownership model won't change, but the implementation will get cleaner.

### 5. Dunning timer — Who runs the escalation clock?

**AR owns dunning.** It has a `dunning_scheduler` service with a full state machine and publishes these events:
- `ar.events.dunning_state_changed`

TrashTech should **drop `dunning_tick`** and subscribe to AR's dunning events instead. The dunning routes in AR properly delegate to the scheduler service (verified in our SoC audit — it's one of the 14 well-structured handler files).

---

## What This Means for Your Migration Plan

Your priority order is correct:

1. **Notifications** (low effort) — Point at platform notification API, configure SendGrid via env vars
2. **AR events** (medium effort) — Subscribe to `ar.events.*`, drop local payment/dunning tracking
3. **AP tip tickets** (medium effort) — Use platform AP with POs + 3-way matching
4. **AR subscriptions** (high effort) — Move billing lifecycle to platform subscriptions module

The platform's commerce modules (AR, subscriptions, payments) are getting refactored for better separation of concerns — see `docs/plans/separation-of-concerns-audit.md` for the full findings. The good news: the modules TrashTech needs most (notifications, AR dunning, AP) are either already clean or being actively fixed.

---

*Full audit report: `docs/plans/separation-of-concerns-audit.md`*
