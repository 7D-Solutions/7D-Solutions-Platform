// ============================================================
// Production smoke spec — TCP UI against deployed production
//
// All production backend ports are firewalled.  This spec runs against the
// TCP UI only (BASE_URL → port 3000, tunnelled via SSH in CI).  BFF routes
// are exercised through the Next.js frontend so no direct backend port access
// is needed.
//
// Requires env vars:
//   BASE_URL            — TCP UI base URL (set by CI via SSH tunnel or public URL)
//   TEST_STAFF_EMAIL    — real production staff email
//   TEST_STAFF_PASSWORD — real production staff password
//
// Run via SSH tunnel:
//   ssh -L 3001:localhost:3000 deploy@prod.7dsolutions.example.com -N &
//   BASE_URL=http://localhost:3001 TEST_STAFF_EMAIL=... TEST_STAFF_PASSWORD=... \
//     npx playwright test tests/e2e/production/smoke.spec.ts
//
// No JWT fallback — must authenticate against real identity-auth.
// ============================================================
import { test, expect, Page } from '@playwright/test';

const STAFF_EMAIL    = process.env.TEST_STAFF_EMAIL    ?? '';
const STAFF_PASSWORD = process.env.TEST_STAFF_PASSWORD ?? '';

// Skip entire suite when BASE_URL is not set or points to localhost without a
// tunnel port (port 3000 on localhost = local dev, not production).
test.skip(
  () => {
    const baseUrl = process.env.BASE_URL ?? '';
    if (!baseUrl) return true;
    // Allow localhost with a non-default port (SSH tunnel: localhost:3001+)
    if (baseUrl.match(/localhost:\d+/) && !baseUrl.includes(':3000')) return false;
    // Allow any non-localhost host
    if (!baseUrl.includes('localhost') && !baseUrl.includes('127.0.0.1')) return false;
    return true;
  },
  'Production smoke requires BASE_URL set to a non-local production TCP UI ' +
    '(or an SSH tunnel: http://localhost:3001). See bead bd-2hxg for setup.',
);

// ── Auth helper (real identity-auth only, no fallback) ────────────────────────
async function loginAsStaffReal(page: Page): Promise<void> {
  if (!STAFF_EMAIL || !STAFF_PASSWORD) {
    throw new Error(
      'TEST_STAFF_EMAIL and TEST_STAFF_PASSWORD must be set for production smoke.',
    );
  }
  const res = await page.request.post('/api/auth/login', {
    data: { email: STAFF_EMAIL, password: STAFF_PASSWORD },
    headers: { 'Content-Type': 'application/json' },
  });
  if (!res.ok()) {
    throw new Error(
      `Production login failed: HTTP ${res.status()} — ` +
        'check TEST_STAFF_EMAIL / TEST_STAFF_PASSWORD and identity-auth health.',
    );
  }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

test.describe('Production smoke — TCP UI', () => {
  test('login page renders on production', async ({ page }) => {
    await page.goto('/login');
    await expect(page.getByRole('heading', { name: /staff login/i })).toBeVisible();
    await expect(page.getByLabel(/email/i)).toBeVisible();
    await expect(page.getByLabel(/password/i)).toBeVisible();
  });

  test('unauthenticated request to /tenants redirects to /login', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page).toHaveURL(/\/login/, { timeout: 10_000 });
  });

  test('staff login succeeds via real identity-auth', async ({ page }) => {
    await loginAsStaffReal(page);
    await page.goto('/tenants');
    await expect(page).not.toHaveURL(/\/login/, { timeout: 10_000 });
  });

  test('tenant list loads from control-plane BFF', async ({ page }) => {
    await loginAsStaffReal(page);
    await page.goto('/tenants');
    await expect(
      page.getByRole('heading', { name: 'Tenants' }),
    ).toBeVisible({ timeout: 15_000 });
    // Either a data table or an empty-state message — but never an error banner
    await expect(page.getByTestId('tenant-list-error')).not.toBeVisible();
  });

  test('BFF /api/tenants returns HTTP 200 on production', async ({ page }) => {
    await loginAsStaffReal(page);
    const res = await page.request.get('/api/tenants');
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(typeof body === 'object' && body !== null).toBeTruthy();
  });

  test('BFF /api/plans returns HTTP 200 on production', async ({ page }) => {
    await loginAsStaffReal(page);
    const res = await page.request.get('/api/plans');
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(typeof body === 'object' && body !== null).toBeTruthy();
  });

  test('plans page renders on production', async ({ page }) => {
    await loginAsStaffReal(page);
    await page.goto('/plans');
    await expect(
      page.getByRole('heading', { name: /plans/i }),
    ).toBeVisible({ timeout: 15_000 });
  });

  test('billing run BFF endpoint is reachable (non-404) on production', async ({ page }) => {
    await loginAsStaffReal(page);
    // POST without a valid body — expect 400 (validation fail) not 404 (route missing)
    const res = await page.request.post('/api/system/run-billing', {
      data: {},
      headers: { 'Content-Type': 'application/json' },
    });
    // 400 = route exists, body validation failed (acceptable)
    // 404 = route missing (failure)
    // 500 = server crash (failure)
    expect(res.status()).not.toBe(404);
    expect(res.status()).not.toBe(500);
  });
});
