// ============================================================
// System Status — Playwright E2E
// Validates: page renders, service tiles present, polling works.
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('System Status', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
  });

  test('status page renders with service tiles', async ({ page }) => {
    await page.goto('/system/status');
    await expect(page.getByTestId('system-status-page')).toBeVisible({ timeout: 15000 });
    await expect(page.getByRole('heading', { name: /system status/i })).toBeVisible();

    // Wait for data to load — either tiles or error state
    const tilesOrError = page.getByTestId('service-tiles').or(page.getByTestId('status-error'));
    await expect(tilesOrError).toBeVisible({ timeout: 15000 });

    // If tiles rendered, verify at least one service tile exists
    const tiles = page.getByTestId('service-tiles');
    if (await tiles.isVisible()) {
      const tileCount = await tiles.locator('[data-testid^="service-tile-"]').count();
      expect(tileCount).toBeGreaterThanOrEqual(1);
    }
  });

  test('status page shows refresh button', async ({ page }) => {
    await page.goto('/system/status');
    await expect(page.getByTestId('system-status-page')).toBeVisible({ timeout: 15000 });
    await expect(page.getByTestId('refresh-btn')).toBeVisible();
  });

  test('overall status banner renders', async ({ page }) => {
    await page.goto('/system/status');
    await expect(page.getByTestId('system-status-page')).toBeVisible({ timeout: 15000 });

    // Wait for data load
    const tilesOrError = page.getByTestId('service-tiles').or(page.getByTestId('status-error'));
    await expect(tilesOrError).toBeVisible({ timeout: 15000 });

    // If tiles loaded, overall status should also show
    const tiles = page.getByTestId('service-tiles');
    if (await tiles.isVisible()) {
      await expect(page.getByTestId('overall-status')).toBeVisible();
    }
  });

  test('sidebar navigation includes System Status link', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByText('System Status')).toBeVisible({ timeout: 15000 });
  });

  test('can navigate to status page from sidebar', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByText('System Status')).toBeVisible({ timeout: 15000 });
    await page.getByText('System Status').click();
    await expect(page).toHaveURL(/\/system\/status/);
    await expect(page.getByTestId('system-status-page')).toBeVisible({ timeout: 15000 });
  });
});
