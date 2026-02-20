// ============================================================
// Idle timeout E2E — warning modal + logout with shortened durations
// Uses window.__TCP_IDLE_MS / __TCP_IDLE_WARN_MS overrides so tests
// run in seconds instead of the production 30-minute timeout.
// Verification: npx playwright test -g "Idle"
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

// Shortened durations for test speed (production = 30min / 5min)
const TEST_IDLE_MS = 8000;      // 8 seconds total idle timeout
const TEST_IDLE_WARN_MS = 4000; // 4 seconds warning window

test.describe('Idle timeout', () => {
  test.beforeEach(async ({ page }) => {
    // Inject shortened idle durations before any page JS executes
    await page.addInitScript(
      ({ idleMs, warnMs }: { idleMs: number; warnMs: number }) => {
        (window as unknown as Record<string, number>).__TCP_IDLE_MS = idleMs;
        (window as unknown as Record<string, number>).__TCP_IDLE_WARN_MS = warnMs;
      },
      { idleMs: TEST_IDLE_MS, warnMs: TEST_IDLE_WARN_MS },
    );
    await loginAsStaff(page);
  });

  test('warning modal appears after idle threshold', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByRole('heading', { name: 'Tenants' })).toBeVisible({ timeout: 10000 });

    // Wait for the warning modal to appear (idle for ~4s)
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: TEST_IDLE_MS });
    await expect(page.getByText(/you've been inactive/i)).toBeVisible();
    await expect(page.getByRole('button', { name: /stay logged in/i })).toBeVisible();
    await expect(page.getByRole('button', { name: /log out now/i })).toBeVisible();
  });

  test('"Stay logged in" resets timer and hides modal', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByRole('heading', { name: 'Tenants' })).toBeVisible({ timeout: 10000 });

    // Wait for warning modal
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: TEST_IDLE_MS });

    // Click "Stay logged in"
    await page.getByRole('button', { name: /stay logged in/i }).click();

    // Modal should close
    await expect(page.getByRole('dialog')).not.toBeVisible();

    // Should still be on the tenants page (not logged out)
    await expect(page).not.toHaveURL(/\/login/);
    await expect(page.getByRole('heading', { name: 'Tenants' })).toBeVisible();
  });

  test('timeout triggers logout after full idle period', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByRole('heading', { name: 'Tenants' })).toBeVisible({ timeout: 10000 });

    // Wait for the full idle timeout to expire (warning + remaining period)
    // The logout redirects to /login
    await expect(page).toHaveURL(/\/login/, { timeout: TEST_IDLE_MS + 5000 });
  });

  test('"Log out now" triggers immediate logout', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByRole('heading', { name: 'Tenants' })).toBeVisible({ timeout: 10000 });

    // Wait for warning modal
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: TEST_IDLE_MS });

    // Click "Log out now"
    await page.getByRole('button', { name: /log out now/i }).click();

    // Should redirect to login
    await expect(page).toHaveURL(/\/login/, { timeout: 10000 });
  });
});
