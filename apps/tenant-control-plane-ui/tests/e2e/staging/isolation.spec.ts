// ============================================================
// Staging isolation spec — Multi-tenant access-control verification
//
// Verifies that tenant-scoped credentials cannot access resources belonging
// to a different tenant via the TCP UI BFF, and that unauthenticated
// requests are rejected on all protected BFF endpoints.
//
// Runs only when BASE_URL points to a non-localhost host (real staging).
//
// Required env vars:
//   BASE_URL              — e.g. http://staging.7dsolutions.example.com:3000
//   ISOLATION_TOKEN_A     — valid tenant-A access_token (non-platform_admin)
//   ISOLATION_TOKEN_B     — valid tenant-B access_token (non-platform_admin)
//   ISOLATION_TENANT_A_ID — UUID of tenant A
//   ISOLATION_TENANT_B_ID — UUID of tenant B
//
// Token generation: Run scripts/staging/isolation_check.sh first; it prints
// the provisioned tenant IDs and can be extended to export tokens as env vars.
//
// Run:
//   BASE_URL=http://... ISOLATION_TOKEN_A=... ISOLATION_TOKEN_B=... \
//   ISOLATION_TENANT_A_ID=... ISOLATION_TENANT_B_ID=... \
//   npx playwright test tests/e2e/staging/isolation.spec.ts
// ============================================================
import { test, expect } from '@playwright/test';

const TOKEN_A   = process.env.ISOLATION_TOKEN_A   ?? '';
const TOKEN_B   = process.env.ISOLATION_TOKEN_B   ?? '';
const TENANT_A  = process.env.ISOLATION_TENANT_A_ID ?? 'tenant-a-uuid';
const TENANT_B  = process.env.ISOLATION_TENANT_B_ID ?? 'tenant-b-uuid';

// Skip entire suite when BASE_URL is localhost (no real staging available)
test.skip(
  () => {
    const baseUrl = process.env.BASE_URL ?? '';
    return !baseUrl || baseUrl.includes('localhost') || baseUrl.includes('127.0.0.1');
  },
  'Isolation spec requires a non-localhost BASE_URL. Set BASE_URL=https://staging.host:3000',
);

// ── Helpers ───────────────────────────────────────────────────────────────────

/**
 * Cookie header value that places a tenant-scoped (non-platform_admin) JWT
 * in the tcp_auth_token slot. The BFF's guardPlatformAdmin() will decode the
 * JWT, find no platform_admin role, and return 403.
 */
function cookieFor(token: string): string {
  return `tcp_auth_token=${token}`;
}

// ── Unauthenticated BFF access (no cookie) ────────────────────────────────────

test.describe('Unauthenticated BFF access — must be denied', () => {
  test('GET /api/tenants without auth returns 401', async ({ request }) => {
    const res = await request.get('/api/tenants');
    expect(res.status()).toBe(401);
  });

  test('GET /api/tenants/{TENANT_B} without auth returns 401', async ({ request }) => {
    const res = await request.get(`/api/tenants/${TENANT_B}`);
    expect(res.status()).toBe(401);
  });

  test('GET /api/tenants/{TENANT_B}/invoices without auth returns 401', async ({ request }) => {
    const res = await request.get(`/api/tenants/${TENANT_B}/invoices`);
    expect(res.status()).toBe(401);
  });

  test('GET /api/plans without auth returns 401', async ({ request }) => {
    const res = await request.get('/api/plans');
    expect(res.status()).toBe(401);
  });

  test('GET /api/tenants/{TENANT_B}/billing/overview without auth returns 401', async ({ request }) => {
    const res = await request.get(`/api/tenants/${TENANT_B}/billing/overview`);
    expect(res.status()).toBe(401);
  });
});

// ── Tenant A reading Tenant B resources (cross-tenant, wrong role) ────────────

test.describe('Tenant A JWT reading Tenant B resources — must be denied (403)', () => {
  test.skip(() => !TOKEN_A, 'ISOLATION_TOKEN_A not set — skipping cross-tenant checks');

  test('tenant-A JWT → GET /api/tenants returns 403', async ({ request }) => {
    const res = await request.get('/api/tenants', {
      headers: { Cookie: cookieFor(TOKEN_A) },
    });
    expect(res.status()).toBe(403);
  });

  test('tenant-A JWT → GET /api/tenants/{TENANT_B} returns 403', async ({ request }) => {
    const res = await request.get(`/api/tenants/${TENANT_B}`, {
      headers: { Cookie: cookieFor(TOKEN_A) },
    });
    expect(res.status()).toBe(403);
  });

  test('tenant-A JWT → GET /api/tenants/{TENANT_B}/invoices returns 403', async ({ request }) => {
    const res = await request.get(`/api/tenants/${TENANT_B}/invoices`, {
      headers: { Cookie: cookieFor(TOKEN_A) },
    });
    expect(res.status()).toBe(403);
  });

  test('tenant-A JWT → GET /api/tenants/{TENANT_B}/billing/overview returns 403', async ({ request }) => {
    const res = await request.get(`/api/tenants/${TENANT_B}/billing/overview`, {
      headers: { Cookie: cookieFor(TOKEN_A) },
    });
    expect(res.status()).toBe(403);
  });

  test('tenant-A JWT → GET /api/plans returns 403', async ({ request }) => {
    const res = await request.get('/api/plans', {
      headers: { Cookie: cookieFor(TOKEN_A) },
    });
    expect(res.status()).toBe(403);
  });
});

// ── Tenant B reading Tenant A resources (cross-tenant, wrong role) ────────────

test.describe('Tenant B JWT reading Tenant A resources — must be denied (403)', () => {
  test.skip(() => !TOKEN_B, 'ISOLATION_TOKEN_B not set — skipping cross-tenant checks');

  test('tenant-B JWT → GET /api/tenants/{TENANT_A} returns 403', async ({ request }) => {
    const res = await request.get(`/api/tenants/${TENANT_A}`, {
      headers: { Cookie: cookieFor(TOKEN_B) },
    });
    expect(res.status()).toBe(403);
  });

  test('tenant-B JWT → GET /api/tenants/{TENANT_A}/invoices returns 403', async ({ request }) => {
    const res = await request.get(`/api/tenants/${TENANT_A}/invoices`, {
      headers: { Cookie: cookieFor(TOKEN_B) },
    });
    expect(res.status()).toBe(403);
  });

  test('tenant-B JWT → GET /api/tenants returns 403', async ({ request }) => {
    const res = await request.get('/api/tenants', {
      headers: { Cookie: cookieFor(TOKEN_B) },
    });
    expect(res.status()).toBe(403);
  });
});

// ── Unauthenticated page routes redirect (not 200) ────────────────────────────

test.describe('UI page routes — unauthenticated must redirect, not expose data', () => {
  test('/tenants page redirects unauthenticated users to /login', async ({ page }) => {
    await page.goto(`/tenants/${TENANT_B}`);
    // Middleware redirects to /login or /forbidden — never serves the page
    await expect(page).toHaveURL(/\/(login|forbidden)/, { timeout: 10_000 });
  });

  test('/plans page redirects unauthenticated users to /login', async ({ page }) => {
    await page.goto('/plans');
    await expect(page).toHaveURL(/\/(login|forbidden)/, { timeout: 10_000 });
  });
});
