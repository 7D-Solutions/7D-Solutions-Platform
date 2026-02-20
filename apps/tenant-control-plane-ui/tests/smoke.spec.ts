// ============================================================
// Smoke test — TCP UI foundation
// Verifies: login flow works, protected routes redirect, tenant list loads.
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('TCP UI smoke', () => {
  test('unauthenticated request to /tenants redirects to login', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page).toHaveURL(/\/login/);
  });

  test('login page renders', async ({ page }) => {
    await page.goto('/login');
    await expect(page.getByRole('heading', { name: /staff login/i })).toBeVisible();
    await expect(page.getByLabel(/email/i)).toBeVisible();
    await expect(page.getByLabel(/password/i)).toBeVisible();
    await expect(page.getByRole('button', { name: /sign in/i })).toBeVisible();
  });

  test('authenticated user can reach /tenants', async ({ page }) => {
    await loginAsStaff(page);
    await page.goto('/tenants');
    // Should not be redirected to login
    await expect(page).not.toHaveURL(/\/login/);
    // Sidebar nav should be visible
    await expect(page.getByText('Tenants')).toBeVisible();
    await expect(page.getByText('Billing')).toBeVisible();
  });

  test('logout clears auth and redirects to login', async ({ page }) => {
    await loginAsStaff(page);
    await page.goto('/tenants');
    // Wait for full hydration before clicking (Button needs React event handler bound)
    await expect(page.getByTestId('user-menu')).toBeVisible({ timeout: 10000 });
    // Use the sidebar logout button
    await page.getByRole('button', { name: /log out/i }).first().click();
    await expect(page).toHaveURL(/\/login/, { timeout: 10000 });
    // Navigating back to protected route should redirect to login again
    await page.goto('/tenants');
    await expect(page).toHaveURL(/\/login/);
  });
});
