// ============================================================
// Hosted Pay Portal E2E — /pay/[session_id]
// Proves the page renders for a real checkout session created
// against the live Payments service (no mocks, no stubs).
//
// Requires: Payments service running at PAYMENTS_BASE_URL (default: http://localhost:8088)
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
    expect(body.status).toBe('pending');
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
});
