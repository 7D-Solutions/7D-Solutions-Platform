// ============================================================
// Tenant List E2E — search, filters, pagination, row/card views
// Verifies: filter bar renders, BFF route is used, UI updates correctly.
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('Tenant List', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    // Clear persisted filter/view state
    await page.request.delete('/api/preferences/view-mode-tenants');
  });

  test('renders tenant list page with header and filter bar', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByRole('heading', { name: 'Tenants' })).toBeVisible();
    await expect(page.getByTestId('filter-bar')).toBeVisible();
    await expect(page.getByTestId('search-input')).toBeVisible();
    await expect(page.getByTestId('status-filter')).toBeVisible();
    await expect(page.getByTestId('plan-filter')).toBeVisible();
  });

  test('fetches tenants via BFF /api/tenants route', async ({ page }) => {
    const bffRequests: string[] = [];

    page.on('request', (req) => {
      const url = req.url();
      if (url.includes('/api/tenants') && !url.includes('/api/tenants/')) {
        bffRequests.push(url);
      }
    });

    await page.goto('/tenants');
    // Wait for the query to fire
    await page.waitForTimeout(1000);

    expect(bffRequests.length).toBeGreaterThan(0);
    expect(bffRequests[0]).toContain('/api/tenants');
  });

  test('search input updates query params', async ({ page }) => {
    const bffRequests: string[] = [];

    page.on('request', (req) => {
      const url = req.url();
      if (url.includes('/api/tenants') && req.method() === 'GET') {
        bffRequests.push(url);
      }
    });

    await page.goto('/tenants');
    await page.waitForTimeout(500);

    // Type a search term
    await page.getByTestId('search-input').fill('acme');

    // Wait for debounce (300ms) + network
    await page.waitForTimeout(1000);

    const searchReqs = bffRequests.filter((u) => u.includes('search=acme'));
    expect(searchReqs.length).toBeGreaterThan(0);
  });

  test('status filter changes query params', async ({ page }) => {
    const bffRequests: string[] = [];

    page.on('request', (req) => {
      const url = req.url();
      if (url.includes('/api/tenants') && req.method() === 'GET') {
        bffRequests.push(url);
      }
    });

    await page.goto('/tenants');
    await page.waitForTimeout(500);

    // Select "Active" status filter
    await page.getByTestId('status-filter').selectOption('active');

    await page.waitForTimeout(500);

    const statusReqs = bffRequests.filter((u) => u.includes('status=active'));
    expect(statusReqs.length).toBeGreaterThan(0);
  });

  test('plan filter changes query params', async ({ page }) => {
    const bffRequests: string[] = [];

    page.on('request', (req) => {
      const url = req.url();
      if (url.includes('/api/tenants') && req.method() === 'GET') {
        bffRequests.push(url);
      }
    });

    await page.goto('/tenants');
    await page.waitForTimeout(500);

    await page.getByTestId('plan-filter').selectOption('Enterprise');

    await page.waitForTimeout(500);

    const planReqs = bffRequests.filter((u) => u.includes('plan=Enterprise'));
    expect(planReqs.length).toBeGreaterThan(0);
  });

  test('row view is default and shows DataTable', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByTestId('row-view')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Row view' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });

  test('switching to card view shows cards', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByTestId('row-view')).toBeVisible();

    await page.getByRole('button', { name: 'Card view' }).click();

    await expect(page.getByTestId('card-view')).toBeVisible();
    await expect(page.getByTestId('row-view')).not.toBeVisible();
  });

  test('column manager toggle is visible in row view', async ({ page }) => {
    await page.goto('/tenants');
    // The DataTable has a "Columns" button for column manager
    await expect(page.getByRole('button', { name: /columns/i })).toBeVisible();
  });

  test('clear filters button appears when filters are active', async ({ page }) => {
    await page.goto('/tenants');

    // Initially no clear button
    await expect(page.getByRole('button', { name: /clear filters/i })).not.toBeVisible();

    // Apply a filter
    await page.getByTestId('status-filter').selectOption('active');

    // Clear button should appear
    await expect(page.getByRole('button', { name: /clear filters/i })).toBeVisible();

    // Click clear
    await page.getByRole('button', { name: /clear filters/i }).click();

    // Filter should reset
    await expect(page.getByTestId('status-filter')).toHaveValue('');
    await expect(page.getByRole('button', { name: /clear filters/i })).not.toBeVisible();
  });

  test('handles empty results gracefully', async ({ page }) => {
    await page.goto('/tenants');

    // The BFF returns empty when upstream is unavailable — verify no crash
    await expect(page.getByRole('heading', { name: 'Tenants' })).toBeVisible();
    // Page should render without error
    await expect(page.locator('body')).toBeVisible();
  });
});
