// ============================================================
// Hosted Pay Portal E2E — /pay/[session_id]
// Proves the page renders for a real checkout session created
// against the live Payments service (no mocks, no stubs).
//
// Also proves:
//   - Status polling endpoint returns correct status after page load
//   - BFF present route is idempotent (200 on repeated calls)
//
// Requires: Payments service running at PAYMENTS_BASE_URL (default: http://localhost:8088)
//
// State machine: created → presented → completed | failed | canceled | expired
// ============================================================
import { test, expect } from '@playwright/test';

const PAYMENTS_BASE_URL = process.env.PAYMENTS_BASE_URL ?? 'http://localhost:8088';

let sessionId: string;

test.describe('Hosted Pay Portal', () => {
  test.beforeAll(async ({ request }) => {
    // Create a real checkout session via the Payments service
    const res = await request.post(`${PAYMENTS_BASE_URL}/api/payments/checkout-sessions`, {
      data: {
        invoice_id: `inv-e2e-${Date.now()}`,
        tenant_id: 'tenant-test-e2e-001',
        amount: 2499,
        currency: 'usd',
        return_url: 'https://example.com/payment/success',
        cancel_url: 'https://example.com/payment/cancel',
      },
      headers: { 'Content-Type': 'application/json' },
    });

    expect(res.ok(), `Failed to create checkout session: ${res.status()}`).toBeTruthy();
    const body = await res.json();
    expect(body.session_id).toBeTruthy();
    sessionId = body.session_id;
  });

  test('pay page renders for valid session', async ({ page }) => {
    await page.goto(`/pay/${sessionId}`);

    // Must NOT redirect to login — this is a public page
    await expect(page).not.toHaveURL(/\/login/);

    // Portal container must be visible
    await expect(page.getByTestId('pay-portal')).toBeVisible({ timeout: 10000 });

    // Amount must be displayed
    await expect(page.getByTestId('pay-amount')).toBeVisible();
    await expect(page.getByTestId('pay-amount')).toContainText('$24.99');
  });

  test('pay page shows payment form', async ({ page }) => {
    await page.goto(`/pay/${sessionId}`);
    await expect(page.getByTestId('pay-portal')).toBeVisible({ timeout: 10000 });

    // Either real Tilled form or mock form must be present
    const hasTilledForm = await page.getByTestId('tilled-payment-form').isVisible();
    const hasMockForm = await page.getByTestId('mock-payment-form').isVisible();
    expect(hasTilledForm || hasMockForm).toBe(true);

    // Pay button must be present
    await expect(page.getByTestId('pay-submit')).toBeVisible();
  });

  test('BFF route returns session data', async ({ request }) => {
    const res = await request.get(`/api/payments/checkout-sessions/${sessionId}`);
    expect(res.ok()).toBeTruthy();

    const body = await res.json();
    expect(body.session_id).toBe(sessionId);
    // After page load (which calls present), status is 'created' or 'presented'
    // depending on whether a previous test visited the page
    expect(['created', 'presented']).toContain(body.status);
    expect(body.amount).toBe(2499);
    expect(body.currency).toBe('usd');
    // client_secret is returned (needed for Tilled.js init)
    expect(body.client_secret).toBeTruthy();
    // return_url / cancel_url stored as provided
    expect(body.return_url).toBe('https://example.com/payment/success');
    expect(body.cancel_url).toBe('https://example.com/payment/cancel');
  });

  test('pay page shows not-found for unknown session', async ({ page }) => {
    await page.goto('/pay/00000000-0000-0000-0000-000000000000');

    // Must NOT redirect to login
    await expect(page).not.toHaveURL(/\/login/);

    // Not-found state must be shown
    await expect(page.getByTestId('pay-not-found')).toBeVisible({ timeout: 10000 });
  });

  test('BFF route rejects invalid session ID', async ({ request }) => {
    const res = await request.get('/api/payments/checkout-sessions/not-a-uuid');
    expect(res.status()).toBe(400);
  });

  test('pay page is accessible without staff JWT', async ({ page }) => {
    // Ensure no auth cookie is set
    await page.context().clearCookies();
    await page.goto(`/pay/${sessionId}`);

    // Should render, not redirect to /login
    await expect(page).not.toHaveURL(/\/login/);
    await expect(page.getByTestId('pay-portal')).toBeVisible({ timeout: 10000 });
  });

  // ── Status polling BFF endpoint ──────────────────────────────────────────

  test('status BFF route returns session_id and status', async ({ request }) => {
    const res = await request.get(`/api/payments/checkout-sessions/${sessionId}/status`);
    expect(res.ok()).toBeTruthy();

    const body = await res.json();
    expect(body.session_id).toBe(sessionId);
    // Status is one of the valid non-terminal states (page may or may not have been visited)
    expect(['created', 'presented']).toContain(body.status);
    // client_secret must NOT be present in the status poll response
    expect(body.client_secret).toBeUndefined();
  });

  test('status BFF route returns 404 for unknown session', async ({ request }) => {
    const res = await request.get(
      '/api/payments/checkout-sessions/00000000-0000-0000-0000-000000000000/status',
    );
    expect(res.status()).toBe(404);
  });

  test('status BFF route rejects invalid session ID', async ({ request }) => {
    const res = await request.get('/api/payments/checkout-sessions/not-a-uuid/status');
    expect(res.status()).toBe(400);
  });

  // ── Present BFF route (idempotency) ─────────────────────────────────────

  test('present BFF route transitions session to presented (idempotent)', async ({
    request,
  }) => {
    // Create a fresh session so we can test the present transition
    const createRes = await request.post(`${PAYMENTS_BASE_URL}/api/payments/checkout-sessions`, {
      data: {
        invoice_id: `inv-e2e-present-${Date.now()}`,
        tenant_id: 'tenant-test-e2e-001',
        amount: 500,
        currency: 'usd',
        return_url: 'https://example.com/payment/success',
        cancel_url: 'https://example.com/payment/cancel',
      },
      headers: { 'Content-Type': 'application/json' },
    });
    expect(createRes.ok()).toBeTruthy();
    const { session_id: freshId } = await createRes.json();

    // Verify initial status is 'created'
    const statusBefore = await request.get(
      `/api/payments/checkout-sessions/${freshId}/status`,
    );
    expect(statusBefore.ok()).toBeTruthy();
    const { status: s1 } = await statusBefore.json();
    expect(s1).toBe('created');

    // First present call
    const p1 = await request.post(`/api/payments/checkout-sessions/${freshId}/present`);
    expect(p1.ok()).toBeTruthy();

    // Status is now 'presented'
    const statusAfter = await request.get(
      `/api/payments/checkout-sessions/${freshId}/status`,
    );
    expect(statusAfter.ok()).toBeTruthy();
    const { status: s2 } = await statusAfter.json();
    expect(s2).toBe('presented');

    // Second present call — idempotent, must also return 200
    const p2 = await request.post(`/api/payments/checkout-sessions/${freshId}/present`);
    expect(p2.ok()).toBeTruthy();

    // Status is still 'presented'
    const statusFinal = await request.get(
      `/api/payments/checkout-sessions/${freshId}/status`,
    );
    expect(statusFinal.ok()).toBeTruthy();
    const { status: s3 } = await statusFinal.json();
    expect(s3).toBe('presented');
  });

  test('present BFF route returns 400 for invalid session ID', async ({ request }) => {
    const res = await request.post('/api/payments/checkout-sessions/not-a-uuid/present');
    expect(res.status()).toBe(400);
  });

  test('present BFF route returns 404 for unknown session', async ({ request }) => {
    const res = await request.post(
      '/api/payments/checkout-sessions/00000000-0000-0000-0000-000000000000/present',
    );
    expect(res.status()).toBe(404);
  });
});
