// ============================================================
// Tenant Overview E2E — list -> detail -> overview path
// Verifies: tenant detail page renders, overview tab shows all
// cards, health snapshot renders deterministically, BFF routes used.
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('Tenant Overview', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    // Clear persisted view state to ensure clean tab state
    await page.request.delete('/api/preferences/view-tenant-detail-home');
  });

  test('navigates from tenant list to detail and renders Overview tab', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByRole('heading', { name: 'Tenants' })).toBeVisible();

    // Navigate to a tenant detail page
    await page.goto('/tenants/test-tenant-001');

    // Verify the detail page rendered
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    // Verify Overview tab is active by default
    await expect(page.getByTestId('tab-overview')).toHaveAttribute('aria-selected', 'true');

    // Verify all four overview cards render
    await expect(page.getByTestId('overview-tab')).toBeVisible();
    await expect(page.getByTestId('status-card')).toBeVisible();
    await expect(page.getByTestId('plan-summary-card')).toBeVisible();
    await expect(page.getByTestId('key-dates-card')).toBeVisible();
    await expect(page.getByTestId('health-snapshot-card')).toBeVisible();
  });

  test('health snapshot renders service rows deterministically', async ({ page }) => {
    // Navigate and wait for the health-snapshot BFF response
    const [response] = await Promise.all([
      page.waitForResponse((res) => res.url().includes('/api/system/health-snapshot')),
      page.goto('/tenants/test-tenant-001'),
    ]);

    expect(response.status()).toBe(200);

    // Wait for React to re-render with the data
    await expect(page.getByTestId('health-snapshot-card')).toBeVisible();
    const healthRows = page.getByTestId('health-service-row');
    await expect(healthRows.first()).toBeVisible({ timeout: 15000 });

    // Verify each expected service is listed
    const healthCard = page.getByTestId('health-snapshot-card');
    await expect(healthCard.getByText('Tenant Registry')).toBeVisible();
    await expect(healthCard.getByText('Plans & Pricing')).toBeVisible();
    await expect(healthCard.getByText('Billing')).toBeVisible();
    await expect(healthCard.getByText('Identity & Auth')).toBeVisible();
  });

  test('BFF routes are called for tenant detail', async ({ page }) => {
    // Wait for all three BFF responses on navigation
    const [tenantRes, planRes, healthRes] = await Promise.all([
      page.waitForResponse((res) =>
        res.url().includes('/api/tenants/test-tenant-001') &&
        !res.url().includes('/plan-summary'),
      ),
      page.waitForResponse((res) => res.url().includes('/plan-summary')),
      page.waitForResponse((res) => res.url().includes('/api/system/health-snapshot')),
      page.goto('/tenants/test-tenant-001'),
    ]);

    expect(tenantRes.status()).toBe(200);
    expect(planRes.status()).toBe(200);
    expect(healthRes.status()).toBe(200);
  });

  test('switching tabs shows placeholder and back to overview', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('overview-tab')).toBeVisible();

    // Click Billing tab
    await page.getByTestId('tab-billing').click();

    // Wait for the UI to update
    await expect(page.getByTestId('tab-placeholder')).toBeVisible({ timeout: 5000 });
    await expect(page.getByTestId('tab-placeholder')).toContainText('Billing');
    await expect(page.getByTestId('overview-tab')).not.toBeVisible();

    // Switch back to Overview
    await page.getByTestId('tab-overview').click();
    await expect(page.getByTestId('overview-tab')).toBeVisible({ timeout: 5000 });
  });

  test('back link navigates to tenant list', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByText('Back to Tenants')).toBeVisible();

    await page.getByText('Back to Tenants').click();
    await expect(page).toHaveURL(/\/tenants$/, { timeout: 10000 });
  });

  test('overview handles partial backend failures gracefully', async ({ page }) => {
    // Navigate to a nonexistent tenant — BFF returns fallback data
    await page.goto('/tenants/nonexistent-tenant-xyz');

    // Page should still render all cards without throwing
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();
    await expect(page.getByTestId('status-card')).toBeVisible();
    await expect(page.getByTestId('plan-summary-card')).toBeVisible();
    await expect(page.getByTestId('key-dates-card')).toBeVisible();
    await expect(page.getByTestId('health-snapshot-card')).toBeVisible();
  });
});
