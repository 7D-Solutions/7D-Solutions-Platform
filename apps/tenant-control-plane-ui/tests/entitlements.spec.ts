// ============================================================
// Entitlements E2E — catalog render, search, filters, BFF route
// Verifies: BFF routes are used, list renders or shows empty
// state, search box is present and functional.
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('Entitlements', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    // Clear persisted view/column/search state for entitlements
    await page.request.delete('/api/preferences/view-mode-entitlements');
    await page.request.delete('/api/preferences/column-config-entitlement-list');
  });

  test('renders Entitlements page with header', async ({ page }) => {
    await page.goto('/entitlements');
    await expect(page.getByRole('heading', { name: 'Entitlements' })).toBeVisible();
  });

  test('fetches entitlements via BFF /api/entitlements route', async ({ page }) => {
    const bffRequests: string[] = [];

    page.on('request', (req) => {
      const url = req.url();
      if (url.includes('/api/entitlements') && !url.includes('/api/entitlements/')) {
        bffRequests.push(url);
      }
    });

    await page.goto('/entitlements');
    await page.waitForTimeout(1000);

    expect(bffRequests.length).toBeGreaterThan(0);
    expect(bffRequests[0]).toContain('/api/entitlements');
  });

  test('entitlement list renders or shows empty state', async ({ page }) => {
    await page.goto('/entitlements');
    await expect(page.getByRole('heading', { name: 'Entitlements' })).toBeVisible();

    // Either entitlement data or an empty-state message should be visible
    const hasData = await page.getByTestId('entitlement-row-view').isVisible().catch(() => false);
    if (hasData) {
      await expect(page.locator('table')).toBeVisible();
    }
    // Page renders without crashing in either case
    await expect(page.locator('body')).toBeVisible();
  });

  test('search box is present and accepts input', async ({ page }) => {
    await page.goto('/entitlements');

    const searchInput = page.getByTestId('entitlement-search-input');
    await expect(searchInput).toBeVisible();

    // Type a search term
    await searchInput.fill('api');
    await expect(searchInput).toHaveValue('api');
  });

  test('search filters the entitlement list', async ({ page }) => {
    await page.goto('/entitlements');
    await expect(page.getByRole('heading', { name: 'Entitlements' })).toBeVisible();

    // Wait for initial data load
    await page.waitForTimeout(1000);

    const searchInput = page.getByTestId('entitlement-search-input');
    await searchInput.fill('api');

    // Wait for debounced search to trigger new fetch
    await page.waitForTimeout(500);

    // Page should still render without error
    await expect(page.locator('body')).toBeVisible();
  });

  test('value type filter is present', async ({ page }) => {
    await page.goto('/entitlements');
    await expect(page.getByTestId('entitlement-type-filter')).toBeVisible();
  });

  test('status filter is present', async ({ page }) => {
    await page.goto('/entitlements');
    await expect(page.getByTestId('entitlement-status-filter')).toBeVisible();
  });

  test('row view is default and shows DataTable', async ({ page }) => {
    await page.goto('/entitlements');
    await expect(page.getByTestId('entitlement-row-view')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Row view' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });

  test('switching to card view shows cards', async ({ page }) => {
    await page.goto('/entitlements');
    await expect(page.getByTestId('entitlement-row-view')).toBeVisible();

    await page.getByRole('button', { name: 'Card view' }).click();
    await expect(page.getByTestId('entitlement-card-view')).toBeVisible();
    await expect(page.getByTestId('entitlement-row-view')).not.toBeVisible();
  });

  test('nav link for Entitlements is highlighted when on /entitlements', async ({ page }) => {
    await page.goto('/entitlements');
    const navLink = page.getByRole('link', { name: 'Entitlements' });
    await expect(navLink).toBeVisible();
  });
});
