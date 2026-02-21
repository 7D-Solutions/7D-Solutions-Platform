// ============================================================
// Production isolation spec — Multi-tenant access-control verification
//
// Verifies that tenant-scoped credentials cannot access resources belonging
// to a different tenant via the TCP UI BFF, and that unauthenticated
// requests are rejected on all protected BFF endpoints.
//
// All production backend ports are firewalled. This spec reaches the BFF
// through the TCP UI Next.js app via SSH tunnel (CI) or a direct public
// URL if available.
//
// Required env vars:
//   BASE_URL              — e.g. http://localhost:3001 (SSH tunnel in CI)
//   ISOLATION_TOKEN_A     — tenant-A access_token (set by isolation_check.sh)
//   ISOLATION_TOKEN_B     — tenant-B access_token (set by isolation_check.sh)
//   ISOLATION_TENANT_A_ID — UUID of tenant A (set by isolation_check.sh)
//   ISOLATION_TENANT_B_ID — UUID of tenant B (set by isolation_check.sh)
//
// In CI: run scripts/production/isolation_check.sh first; it writes tenant
// IDs and tokens to $GITHUB_ENV for subsequent Playwright steps.
//
// Run locally via SSH tunnel:
//   ssh -L 3001:localhost:3000 deploy@prod.7dsolutions.example.com -N &
//   BASE_URL=http://localhost:3001 \
//   ISOLATION_TOKEN_A=... ISOLATION_TOKEN_B=... \
//   ISOLATION_TENANT_A_ID=... ISOLATION_TENANT_B_ID=... \
//   npx playwright test tests/e2e/production/isolation.spec.ts
// ============================================================
import { test, expect } from '@playwright/test';

const TOKEN_A  = process.env.ISOLATION_TOKEN_A    ?? '';
const TOKEN_B  = process.env.ISOLATION_TOKEN_B    ?? '';
const TENANT_A = process.env.ISOLATION_TENANT_A_ID ?? 'tenant-a-uuid';
const TENANT_B = process.env.ISOLATION_TENANT_B_ID ?? 'tenant-b-uuid';

// Skip entire suite when BASE_URL is not set, or points to localhost:3000 (local dev).
// Allow localhost with a non-default port — this is the SSH tunnel pattern (e.g. localhost:3001).
test.skip(
  () => {
    const baseUrl = process.env.BASE_URL ?? '';
    if (!baseUrl) return true;
    // Allow localhost only with non-default port (SSH tunnel: localhost:3001+)
    if (baseUrl.match(/localhost:\d+/) && !baseUrl.includes(':3000')) return false;
    // Allow any non-localhost host
    if (!baseUrl.includes('localhost') && !baseUrl.includes('127.0.0.1')) return false;
    return true;
  },
  'Production isolation spec requires a non-local BASE_URL or SSH tunnel ' +
    '(http://localhost:3001+). Run isolation_check.sh first to provision tenants.',
);

// ── Helper ────────────────────────────────────────────────────────────────────

function cookieFor(token: string): string {
  return `tcp_auth_token=${token}`;
}

// ── Unauthenticated BFF access — must be denied ───────────────────────────────

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
  test.skip(() => !TOKEN_A, 'ISOLATION_TOKEN_A not set — run isolation_check.sh first');

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
  test.skip(() => !TOKEN_B, 'ISOLATION_TOKEN_B not set — run isolation_check.sh first');

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

// ── Unauthenticated UI page routes — must redirect, never expose data ─────────

test.describe('UI page routes — unauthenticated must redirect, not expose data', () => {
  test('/tenants page redirects unauthenticated users to /login', async ({ page }) => {
    await page.goto(`/tenants/${TENANT_B}`);
    await expect(page).toHaveURL(/\/(login|forbidden)/, { timeout: 10_000 });
  });

  test('/plans page redirects unauthenticated users to /login', async ({ page }) => {
    await page.goto('/plans');
    await expect(page).toHaveURL(/\/(login|forbidden)/, { timeout: 10_000 });
  });
});
