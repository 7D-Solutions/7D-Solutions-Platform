// ============================================================
// Smoke test — TCP UI foundation
// Verifies: login flow works, /app/** protected, tenant list loads.
// Requires: identity-auth running, TEST_STAFF_EMAIL/PASSWORD set.
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('TCP UI smoke', () => {
  test('unauthenticated request to /app/tenants redirects to login', async ({ page }) => {
    await page.goto('/app/tenants');
    await expect(page).toHaveURL(/\/app\/login/);
  });

  test('login page renders', async ({ page }) => {
    await page.goto('/app/login');
    await expect(page.getByRole('heading', { name: /staff login/i })).toBeVisible();
    await expect(page.getByLabel(/email/i)).toBeVisible();
    await expect(page.getByLabel(/password/i)).toBeVisible();
    await expect(page.getByRole('button', { name: /sign in/i })).toBeVisible();
  });

  test('authenticated user can reach /app/tenants', async ({ page }) => {
    await loginAsStaff(page);
    await page.goto('/app/tenants');
    // Should not be redirected to login
    await expect(page).not.toHaveURL(/\/app\/login/);
    // Sidebar nav should be visible
    await expect(page.getByText('Tenants')).toBeVisible();
    await expect(page.getByText('Billing')).toBeVisible();
  });

  test('logout clears auth and redirects to login', async ({ page }) => {
    await loginAsStaff(page);
    await page.goto('/app/tenants');
    await page.getByRole('button', { name: /log out/i }).click();
    await expect(page).toHaveURL(/\/app\/login/);
    // Navigating back to /app/** should redirect to login again
    await page.goto('/app/tenants');
    await expect(page).toHaveURL(/\/app\/login/);
  });
});
