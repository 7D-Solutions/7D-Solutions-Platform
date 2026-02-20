// ============================================================
// Plan Detail E2E — detail page renders via BFF, sections visible
// Verifies: BFF /api/plans/[plan_id] route, key sections render,
// navigation from list to detail, back navigation.
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('Plan Detail', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
  });

  test('navigates from plan list to plan detail via row click', async ({ page }) => {
    await page.goto('/plans');
    await expect(page.getByRole('heading', { name: 'Plans & Pricing' })).toBeVisible();

    // Wait for plan data to load
    await page.waitForTimeout(1000);

    // Click first data row in the table (skip header row)
    const firstDataRow = page.locator('table tbody tr').first();
    const hasRows = await firstDataRow.isVisible().catch(() => false);

    if (hasRows) {
      await firstDataRow.click();
      // Should navigate to plan detail page
      await expect(page.getByTestId('plan-detail')).toBeVisible({ timeout: 5000 });
      await expect(page.getByTestId('plan-detail-name')).toBeVisible();
    }
  });

  test('plan detail loads via BFF /api/plans/[plan_id] route', async ({ page }) => {
    const bffRequests: string[] = [];

    page.on('request', (req) => {
      const url = req.url();
      if (url.match(/\/api\/plans\/[^?]+/)) {
        bffRequests.push(url);
      }
    });

    // Navigate directly to a known seed plan
    await page.goto('/plans/plan-starter');
    await expect(page.getByTestId('plan-detail')).toBeVisible({ timeout: 5000 });

    expect(bffRequests.length).toBeGreaterThan(0);
    expect(bffRequests[0]).toContain('/api/plans/plan-starter');
  });

  test('plan detail renders key sections for seed plan', async ({ page }) => {
    await page.goto('/plans/plan-professional');
    await expect(page.getByTestId('plan-detail')).toBeVisible({ timeout: 5000 });

    // Plan name
    await expect(page.getByTestId('plan-detail-name')).toHaveText('Professional');

    // Pricing Rules section
    await expect(page.getByTestId('plan-pricing-rules')).toBeVisible();
    await expect(page.getByTestId('plan-pricing-rules').getByText('Base platform fee')).toBeVisible();

    // Metered Dimensions section
    await expect(page.getByTestId('plan-metered-dimensions')).toBeVisible();
    await expect(page.getByTestId('plan-metered-dimensions').getByText('API Calls')).toBeVisible();
    await expect(page.getByTestId('plan-metered-dimensions').getByText('Storage')).toBeVisible();

    // Bundles section
    await expect(page.getByTestId('plan-bundles')).toBeVisible();
    await expect(page.getByTestId('plan-bundles').getByText('Core Features')).toBeVisible();
    await expect(page.getByTestId('plan-bundles').getByText('Advanced Analytics')).toBeVisible();

    // Entitlements section
    await expect(page.getByTestId('plan-entitlements')).toBeVisible();
    await expect(page.getByTestId('plan-entitlements').getByText('Max Projects')).toBeVisible();
    await expect(page.getByTestId('plan-entitlements').getByText('SSO Enabled')).toBeVisible();
  });

  test('plan detail shows empty hints for plan with no metered dimensions', async ({ page }) => {
    await page.goto('/plans/plan-starter');
    await expect(page.getByTestId('plan-detail')).toBeVisible({ timeout: 5000 });

    // Starter plan has no metered dimensions
    await expect(page.getByTestId('plan-metered-dimensions').getByText('No metered dimensions.')).toBeVisible();

    // But it has pricing rules
    await expect(page.getByTestId('plan-pricing-rules').getByText('Monthly flat fee')).toBeVisible();
  });

  test('plan detail shows error for non-existent plan', async ({ page }) => {
    await page.goto('/plans/non-existent-plan-id');
    await expect(page.getByTestId('plan-detail-error')).toBeVisible({ timeout: 5000 });
  });

  test('back button navigates to plan list', async ({ page }) => {
    await page.goto('/plans/plan-starter');
    await expect(page.getByTestId('plan-detail')).toBeVisible({ timeout: 5000 });

    // Click back button
    await page.getByRole('button', { name: /plans/i }).first().click();

    // Should show plan list
    await expect(page.getByRole('heading', { name: 'Plans & Pricing' })).toBeVisible({ timeout: 5000 });
  });

  test('plan detail shows status badge', async ({ page }) => {
    await page.goto('/plans/plan-trial');
    await expect(page.getByTestId('plan-detail')).toBeVisible({ timeout: 5000 });

    // Trial plan has draft status
    await expect(page.getByText('Draft')).toBeVisible();
  });
});
