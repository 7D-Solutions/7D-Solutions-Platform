# Tenant Onboarding Runbook

## Purpose

Step-by-step guide for provisioning a new tenant on the 7D Solutions Platform.
Covers the recommended path via the Tenant Control Plane UI and recovery procedures for each failure scenario.

## Recommended Path: TCP UI Wizard

**Preferred method.** The TCP UI wizard is the single source of truth for operator-driven onboarding.
No direct database edits are required or supported.

### Prerequisites

- Platform admin account with the `platform_admin` role
- Access to the Tenant Control Plane UI (TCP UI)
- At least one active plan in the plan catalog

### Procedure

1. Navigate to **Tenants → New Tenant** in the TCP UI
2. **Step 1 — Tenant details**: Enter a name and select the environment (`development`, `staging`, or `production`)
3. **Step 2 — Plan**: Select the billing plan from the catalog (active plans only)
4. **Step 3 — Admin user**: Enter the initial administrator email and password (minimum 8 characters)
5. Click **Create Tenant**
6. On success, click **Go to tenant** to verify the tenant was provisioned correctly

The wizard enforces sequence: step 2 is locked until step 1 is complete, step 3 is locked until step 2 is complete.
All calls go through the BFF layer — no direct calls to Rust services are made from the browser.

## Verification

After onboarding, confirm the tenant is functional:

```bash
# Show tenant state via tenantctl
cargo run -p tenantctl -- tenant show --tenant <TENANT_ID>

# Verify module health
cargo run -p tenantctl -- tenant verify --tenant <TENANT_ID>
```

Expected: status `active`, all modules green.

## Recovery Procedures

### Step 1 Fails — Tenant record not created

The registry API returned an error. The tenant does not exist in any system.

**Recovery:** Retry the wizard from the beginning. No cleanup required.

### Step 2 Fails — Plan not available

The plan catalog returned an error or no active plans exist.

**Recovery:**
1. Check the plan catalog service is running: `docker compose ps ttp`
2. Ensure at least one plan has status `active` in the catalog
3. Retry the wizard — step 1 data is preserved client-side during the session

### Step 3 Fails — Admin user not created (partial tenant)

The tenant record was created (step 1 succeeded) but user provisioning failed (identity-auth error).

The TCP UI will show a warning notification and redirect to the tenant detail page.

**Recovery via TCP UI:**
1. Navigate to **Tenants → [tenant name] → Users**
2. Use the access management panel to add the initial admin user
3. If the access panel is unavailable, use `tenantctl` CLI as a fallback:

```bash
# tenantctl does not directly create users — use identity-auth admin API
curl -X POST http://localhost:8080/api/auth/register \
  -H 'Content-Type: application/json' \
  -d '{
    "tenant_id": "<TENANT_ID>",
    "user_id": "<NEW_UUID>",
    "email": "admin@tenant.com",
    "password": "SecurePassword!"
  }'
```

4. Verify the user can log in before considering the tenant fully provisioned

### Tenant stuck in partial state (no plan, no users)

If a tenant was created but is missing required configuration:

1. **Check current state:** TCP UI → Tenants → [tenant] → Billing and Users tabs
2. **Assign plan:** Use the Billing tab to assign or change the plan
3. **Add admin user:** Use the Users / Access tab
4. **Verify:** Run `tenantctl tenant verify --tenant <TENANT_ID>` to confirm all modules healthy
5. **Activate if needed:** If status is `provisioning`, run `tenantctl tenant activate --tenant <TENANT_ID>`

## Fallback: tenantctl CLI

The `tenantctl` CLI provisions module databases and runs migrations. Use it only as a recovery tool — not for standard onboarding.

```bash
# Provision module databases (runs migrations)
cargo run -p tenantctl -- tenant create --tenant <TENANT_ID>

# Activate the tenant
cargo run -p tenantctl -- tenant activate --tenant <TENANT_ID>
```

**Note:** `tenantctl create` provisions infrastructure only. You must still assign a plan and create the admin user via the TCP UI or the identity-auth admin API.

## No Direct DB Edits

Direct database edits are not supported for onboarding. All tenant records must be created through the tenant-registry API (via TCP UI or BFF), and all user records through the identity-auth service. This ensures audit logs, event bus messages, and module state remain consistent.
