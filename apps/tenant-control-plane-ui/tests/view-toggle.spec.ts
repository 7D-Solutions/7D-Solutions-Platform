// ============================================================
// View Toggle E2E — row/card toggle with BFF-persisted preference
// Verifies: toggle switches view, preference persists across reload.
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('View Toggle', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
  });

  test('defaults to row view on first load', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByTestId('row-view')).toBeVisible();
    await expect(page.getByTestId('card-view')).not.toBeVisible();
    // List button should be pressed
    await expect(page.getByRole('button', { name: 'Row view' })).toHaveAttribute('aria-pressed', 'true');
    await expect(page.getByRole('button', { name: 'Card view' })).toHaveAttribute('aria-pressed', 'false');
  });

  test('toggle switches from row to card view', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByTestId('row-view')).toBeVisible();

    await page.getByRole('button', { name: 'Card view' }).click();

    await expect(page.getByTestId('card-view')).toBeVisible();
    await expect(page.getByTestId('row-view')).not.toBeVisible();
    await expect(page.getByRole('button', { name: 'Card view' })).toHaveAttribute('aria-pressed', 'true');
  });

  test('toggle switches from card back to row view', async ({ page }) => {
    await page.goto('/tenants');

    await page.getByRole('button', { name: 'Card view' }).click();
    await expect(page.getByTestId('card-view')).toBeVisible();

    await page.getByRole('button', { name: 'Row view' }).click();
    await expect(page.getByTestId('row-view')).toBeVisible();
    await expect(page.getByTestId('card-view')).not.toBeVisible();
  });

  test('preference persists across page reload', async ({ page }) => {
    await page.goto('/tenants');

    // Switch to card view
    await page.getByRole('button', { name: 'Card view' }).click();
    await expect(page.getByTestId('card-view')).toBeVisible();

    // Wait for BFF save to complete (debounce is 1s)
    await page.waitForTimeout(1500);

    // Reload the page
    await page.reload();

    // Card view should persist
    await expect(page.getByTestId('card-view')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Card view' })).toHaveAttribute('aria-pressed', 'true');
  });

  test('preference persists across navigation', async ({ page }) => {
    await page.goto('/tenants');

    // Switch to card view
    await page.getByRole('button', { name: 'Card view' }).click();
    await expect(page.getByTestId('card-view')).toBeVisible();

    // Wait for BFF save
    await page.waitForTimeout(1500);

    // Navigate away and back
    await page.goto('/tenants');

    // Card view should persist
    await expect(page.getByTestId('card-view')).toBeVisible();
  });
});
