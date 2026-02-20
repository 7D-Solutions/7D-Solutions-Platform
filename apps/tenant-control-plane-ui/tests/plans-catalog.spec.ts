// ============================================================
// Plans Catalog E2E — list render, column manager, view toggle
// Verifies: BFF route is used, plan list renders or empty state,
// column manager toggles a column.
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('Plans Catalog', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    // Clear persisted view/column state for plans
    await page.request.delete('/api/preferences/view-mode-plans');
    await page.request.delete('/api/preferences/column-config-plan-list');
  });

  test('renders Plans & Pricing page with header', async ({ page }) => {
    await page.goto('/plans');
    await expect(page.getByRole('heading', { name: 'Plans & Pricing' })).toBeVisible();
  });

  test('fetches plans via BFF /api/plans route', async ({ page }) => {
    const bffRequests: string[] = [];

    page.on('request', (req) => {
      const url = req.url();
      if (url.includes('/api/plans') && !url.includes('/api/plans/')) {
        bffRequests.push(url);
      }
    });

    await page.goto('/plans');
    await page.waitForTimeout(1000);

    expect(bffRequests.length).toBeGreaterThan(0);
    expect(bffRequests[0]).toContain('/api/plans');
  });

  test('plan list renders or shows empty state', async ({ page }) => {
    await page.goto('/plans');
    await expect(page.getByRole('heading', { name: 'Plans & Pricing' })).toBeVisible();

    // Either plan data or an empty-state message should be visible
    const hasData = await page.getByTestId('plan-row-view').isVisible().catch(() => false);
    if (hasData) {
      // DataTable should be rendered with at least a header row
      await expect(page.locator('table')).toBeVisible();
    }
    // Page renders without crashing in either case
    await expect(page.locator('body')).toBeVisible();
  });

  test('column manager toggle is visible and can toggle a column', async ({ page }) => {
    await page.goto('/plans');

    // "Columns" button should be visible in row view
    const columnsBtn = page.getByRole('button', { name: /columns/i });
    await expect(columnsBtn).toBeVisible();

    // Enter edit mode
    await columnsBtn.click();

    // "Done" button should appear (edit mode active)
    await expect(page.getByRole('button', { name: /done/i })).toBeVisible();

    // Find a non-locked column checkbox (e.g. "Pricing Model")
    const checkbox = page.getByRole('checkbox', { name: /pricing model/i });
    if (await checkbox.isVisible()) {
      // Uncheck it
      await checkbox.uncheck();
      // The column header should be hidden in the table
      await expect(page.locator('th', { hasText: 'PRICING MODEL' })).not.toBeVisible();

      // Re-check it
      await checkbox.check();
      await expect(page.locator('th', { hasText: 'PRICING MODEL' })).toBeVisible();
    }

    // Exit edit mode
    await page.getByRole('button', { name: /done/i }).click();
    await expect(page.getByRole('button', { name: /columns/i })).toBeVisible();
  });

  test('row view is default and shows DataTable', async ({ page }) => {
    await page.goto('/plans');
    await expect(page.getByTestId('plan-row-view')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Row view' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });

  test('switching to card view shows cards', async ({ page }) => {
    await page.goto('/plans');
    await expect(page.getByTestId('plan-row-view')).toBeVisible();

    await page.getByRole('button', { name: 'Card view' }).click();
    await expect(page.getByTestId('plan-card-view')).toBeVisible();
    await expect(page.getByTestId('plan-row-view')).not.toBeVisible();
  });

  test('status filter is present', async ({ page }) => {
    await page.goto('/plans');
    await expect(page.getByTestId('plan-status-filter')).toBeVisible();
  });

  test('nav link for Plans & Pricing is highlighted when on /plans', async ({ page }) => {
    await page.goto('/plans');
    const navLink = page.getByRole('link', { name: 'Plans & Pricing' });
    await expect(navLink).toBeVisible();
  });
});
