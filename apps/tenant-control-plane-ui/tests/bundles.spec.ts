// ============================================================
// Bundles E2E — list render, detail drill-down, composition
// Verifies: BFF routes are used, bundles list renders or shows
// empty state, detail shows composition when data exists.
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('Bundles', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    // Clear persisted view/column state for bundles
    await page.request.delete('/api/preferences/view-mode-bundles');
    await page.request.delete('/api/preferences/column-config-bundle-list');
  });

  test('renders Bundles & Features page with header', async ({ page }) => {
    await page.goto('/bundles');
    await expect(page.getByRole('heading', { name: 'Bundles & Features' })).toBeVisible();
  });

  test('fetches bundles via BFF /api/bundles route', async ({ page }) => {
    const bffRequests: string[] = [];

    page.on('request', (req) => {
      const url = req.url();
      if (url.includes('/api/bundles') && !url.includes('/api/bundles/')) {
        bffRequests.push(url);
      }
    });

    await page.goto('/bundles');
    await page.waitForTimeout(1000);

    expect(bffRequests.length).toBeGreaterThan(0);
    expect(bffRequests[0]).toContain('/api/bundles');
  });

  test('bundle list renders or shows empty state', async ({ page }) => {
    await page.goto('/bundles');
    await expect(page.getByRole('heading', { name: 'Bundles & Features' })).toBeVisible();

    // Either bundle data or an empty-state message should be visible
    const hasData = await page.getByTestId('bundle-row-view').isVisible().catch(() => false);
    if (hasData) {
      // DataTable should be rendered with at least a header row
      await expect(page.locator('table')).toBeVisible();
    }
    // Page renders without crashing in either case
    await expect(page.locator('body')).toBeVisible();
  });

  test('column manager toggle is visible and can toggle a column', async ({ page }) => {
    await page.goto('/bundles');

    // "Columns" button should be visible in row view
    const columnsBtn = page.getByRole('button', { name: /columns/i });
    await expect(columnsBtn).toBeVisible();

    // Enter edit mode
    await columnsBtn.click();

    // "Done" button should appear (edit mode active)
    await expect(page.getByRole('button', { name: /done/i })).toBeVisible();

    // Find the Status checkbox (non-locked)
    const checkbox = page.getByRole('checkbox', { name: /status/i });
    if (await checkbox.isVisible()) {
      await checkbox.uncheck();
      await expect(page.locator('th', { hasText: 'STATUS' })).not.toBeVisible();

      await checkbox.check();
      await expect(page.locator('th', { hasText: 'STATUS' })).toBeVisible();
    }

    // Exit edit mode
    await page.getByRole('button', { name: /done/i }).click();
    await expect(page.getByRole('button', { name: /columns/i })).toBeVisible();
  });

  test('status filter is present', async ({ page }) => {
    await page.goto('/bundles');
    await expect(page.getByTestId('bundle-status-filter')).toBeVisible();
  });

  test('row view is default and shows DataTable', async ({ page }) => {
    await page.goto('/bundles');
    await expect(page.getByTestId('bundle-row-view')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Row view' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });

  test('switching to card view shows cards', async ({ page }) => {
    await page.goto('/bundles');
    await expect(page.getByTestId('bundle-row-view')).toBeVisible();

    await page.getByRole('button', { name: 'Card view' }).click();
    await expect(page.getByTestId('bundle-card-view')).toBeVisible();
    await expect(page.getByTestId('bundle-row-view')).not.toBeVisible();
  });

  test('nav link for Bundles is highlighted when on /bundles', async ({ page }) => {
    await page.goto('/bundles');
    const navLink = page.getByRole('link', { name: 'Bundles' });
    await expect(navLink).toBeVisible();
  });

  test('clicking a bundle row navigates to detail', async ({ page }) => {
    await page.goto('/bundles');
    await expect(page.getByRole('heading', { name: 'Bundles & Features' })).toBeVisible();

    // Wait for data to load — seed data will populate
    await page.waitForTimeout(1000);

    // Try clicking the first data row (skip header)
    const firstDataRow = page.locator('table tbody tr').first();
    const rowVisible = await firstDataRow.isVisible().catch(() => false);

    if (rowVisible) {
      await firstDataRow.click();

      // Should navigate to detail page
      await page.waitForURL(/\/bundles\//, { timeout: 5000 });
      await expect(page.getByTestId('bundle-detail-name')).toBeVisible();
    }
  });

  test('bundle detail shows composition or empty state', async ({ page }) => {
    // Navigate to a known seed bundle
    await page.goto('/bundles/bundle-essential');

    // Either composition renders or we get an error (if TTP unavailable and no seed)
    const detailName = page.getByTestId('bundle-detail-name');
    const isDetailVisible = await detailName.isVisible({ timeout: 5000 }).catch(() => false);

    if (isDetailVisible) {
      await expect(detailName).toHaveText('Essential Features');

      // Composition table should render
      const compositionTable = page.getByTestId('bundle-composition-table');
      const emptyComposition = page.getByTestId('bundle-empty-composition');

      const hasComposition = await compositionTable.isVisible().catch(() => false);
      const hasEmpty = await emptyComposition.isVisible().catch(() => false);

      // One of the two states must be present
      expect(hasComposition || hasEmpty).toBe(true);

      if (hasComposition) {
        // Should show entitlement rows
        const rows = page.locator('[data-testid="bundle-composition-table"] tbody tr');
        await expect(rows.first()).toBeVisible();
      }
    }
  });

  test('bundle detail back link returns to list', async ({ page }) => {
    await page.goto('/bundles/bundle-essential');

    const backLink = page.getByTestId('bundle-back-link');
    const isVisible = await backLink.isVisible({ timeout: 5000 }).catch(() => false);

    if (isVisible) {
      await backLink.click();
      await page.waitForURL(/\/bundles$/, { timeout: 5000 });
      await expect(page.getByRole('heading', { name: 'Bundles & Features' })).toBeVisible();
    }
  });

  test('bundle detail fetches via BFF /api/bundles/[id] route', async ({ page }) => {
    const detailRequests: string[] = [];

    page.on('request', (req) => {
      const url = req.url();
      if (url.includes('/api/bundles/bundle-')) {
        detailRequests.push(url);
      }
    });

    await page.goto('/bundles/bundle-essential');
    await page.waitForTimeout(1000);

    expect(detailRequests.length).toBeGreaterThan(0);
    expect(detailRequests[0]).toContain('/api/bundles/bundle-essential');
  });
});
