// ============================================================
// Tenant Billing Overview E2E — navigate to Billing tab,
// verify cards render (data or "Not available"), BFF called.
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('Tenant Billing Overview', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    // Clear persisted view state to ensure clean tab state
    await page.request.delete('/api/preferences/view-tenant-detail-home');
  });

  test('renders Billing tab with all overview cards', async ({ page }) => {
    // Navigate to tenant detail and wait for the billing BFF response
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    // Click Billing tab
    await page.getByTestId('tab-billing').click();
    await expect(page.getByTestId('tab-billing')).toHaveAttribute('aria-selected', 'true');

    // Wait for the billing overview BFF call
    await expect(page.getByTestId('billing-tab')).toBeVisible({ timeout: 15000 });

    // All five cards should render (with data or "Not available")
    await expect(page.getByTestId('billing-charges-card')).toBeVisible();
    await expect(page.getByTestId('billing-last-invoice-card')).toBeVisible();
    await expect(page.getByTestId('billing-outstanding-card')).toBeVisible();
    await expect(page.getByTestId('billing-payment-card')).toBeVisible();
    await expect(page.getByTestId('billing-dunning-card')).toBeVisible();
  });

  test('BFF billing overview route is called', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    // Set up response listener before clicking the tab
    const billingResponsePromise = page.waitForResponse(
      (res) => res.url().includes('/billing/overview'),
    );

    await page.getByTestId('tab-billing').click();

    const billingRes = await billingResponsePromise;
    expect(billingRes.status()).toBe(200);

    // Verify the response has the expected shape (all sections present)
    const body = await billingRes.json();
    expect(body).toHaveProperty('charges');
    expect(body).toHaveProperty('last_invoice');
    expect(body).toHaveProperty('outstanding');
    expect(body).toHaveProperty('payment_status');
    expect(body).toHaveProperty('dunning');

    // Each section should have an availability flag
    expect(body.charges).toHaveProperty('availability');
    expect(body.last_invoice).toHaveProperty('availability');
    expect(body.outstanding).toHaveProperty('availability');
    expect(body.payment_status).toHaveProperty('availability');
    expect(body.dunning).toHaveProperty('availability');
  });

  test('unavailable sections show Not available label', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-billing').click();
    await expect(page.getByTestId('billing-tab')).toBeVisible({ timeout: 15000 });

    // When upstream services are down, sections show "Not available" instead of zeros
    // At least some sections should render — either with data or with "Not available"
    const cards = page.getByTestId('billing-tab').locator('[data-testid$="-card"]');
    await expect(cards.first()).toBeVisible();

    // Verify that unavailable sections use the explicit label, not blank or zeros
    const unavailableMarkers = page.getByTestId('section-unavailable');
    const cardCount = await cards.count();

    // All cards should have rendered (5 total)
    expect(cardCount).toBe(5);

    // Each unavailable section should say "Not available"
    const unavailableCount = await unavailableMarkers.count();
    for (let i = 0; i < unavailableCount; i++) {
      await expect(unavailableMarkers.nth(i)).toContainText('Not available');
    }
  });

  test('Billing tab handles complete BFF failure gracefully', async ({ page }) => {
    // Navigate to a nonexistent tenant — upstream calls will fail
    await page.goto('/tenants/nonexistent-tenant-xyz');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-billing').click();
    await expect(page.getByTestId('billing-tab')).toBeVisible({ timeout: 15000 });

    // All cards should still render (with "Not available" labels)
    await expect(page.getByTestId('billing-charges-card')).toBeVisible();
    await expect(page.getByTestId('billing-last-invoice-card')).toBeVisible();
    await expect(page.getByTestId('billing-outstanding-card')).toBeVisible();
    await expect(page.getByTestId('billing-payment-card')).toBeVisible();
    await expect(page.getByTestId('billing-dunning-card')).toBeVisible();
  });

  test('switching from Billing back to Overview works', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('overview-tab')).toBeVisible();

    // Switch to Billing
    await page.getByTestId('tab-billing').click();
    await expect(page.getByTestId('billing-tab')).toBeVisible({ timeout: 15000 });
    await expect(page.getByTestId('overview-tab')).not.toBeVisible();

    // Switch back to Overview
    await page.getByTestId('tab-overview').click();
    await expect(page.getByTestId('overview-tab')).toBeVisible({ timeout: 5000 });
    await expect(page.getByTestId('billing-tab')).not.toBeVisible();
  });
});
