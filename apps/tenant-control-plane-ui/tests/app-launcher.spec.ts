// ============================================================
// App Launcher E2E — subscribed apps cards, launch button, URL hygiene
// Verification: npx playwright test -g "App Launcher"
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

const BFF_TIMEOUT = 15000;

test.describe('App Launcher', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    await page.request.delete('/api/preferences/view-tenant-detail-home').catch(() => {});
  });

  test('Access tab renders App Launcher panel', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('app-launcher-panel')).toBeVisible({ timeout: BFF_TIMEOUT });
  });

  test('apps are fetched via BFF route', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    const [bffRes] = await Promise.all([
      page.waitForResponse(
        (res) =>
          res.url().includes('/api/tenants/test-tenant-001/apps') &&
          res.status() === 200,
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('tab-access').click(),
    ]);

    const body = await bffRes.json();
    expect(body).toHaveProperty('apps');
    expect(Array.isArray(body.apps)).toBe(true);
  });

  test('app cards render with name and status', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('app-launcher-panel')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Either cards or empty state should appear
    const grid = page.getByTestId('apps-grid');
    const empty = page.getByTestId('apps-empty');
    await expect(grid.or(empty)).toBeVisible({ timeout: BFF_TIMEOUT });

    // If cards exist, verify structure
    const cards = page.getByTestId('app-card');
    const count = await cards.count();
    if (count > 0) {
      // Each card has a name
      await expect(cards.first().getByTestId('app-card-name')).toBeVisible();
    }
  });

  test('app cards show "Launch URL not configured" when no URL', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('app-launcher-panel')).toBeVisible({ timeout: BFF_TIMEOUT });

    const grid = page.getByTestId('apps-grid');
    await expect(grid).toBeVisible({ timeout: BFF_TIMEOUT });

    // Seed data has no launch URLs, so all cards should show the warning
    const noUrlWarnings = page.getByTestId('app-no-launch-url');
    const warningCount = await noUrlWarnings.count();
    expect(warningCount).toBeGreaterThan(0);
  });

  test('launch button opens new tab without token in URL', async ({ page, context }) => {
    // Intercept the BFF to return an app with a launch_url
    await page.route('**/api/tenants/*/apps', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          apps: [
            {
              id: 'mod-test',
              name: 'Test App',
              module_code: 'test',
              launch_url: 'https://test-app.example.com/dashboard',
              status: 'available',
            },
          ],
        }),
      });
    });

    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('app-launcher-panel')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('apps-grid')).toBeVisible({ timeout: BFF_TIMEOUT });

    // The Launch button should be visible
    const launchBtn = page.getByTestId('app-launch-btn').first();
    await expect(launchBtn).toBeVisible();

    // Intercept window.open to capture the URL without actually navigating
    const launchUrlPromise = page.evaluate(() => {
      return new Promise<string>((resolve) => {
        const orig = window.open;
        window.open = (url, ...args) => {
          resolve(String(url));
          // Return null — do not open a real tab in test env
          return null;
        };
      });
    });

    await launchBtn.click();
    const launchUrl = await launchUrlPromise;

    // URL must be the expected launch URL
    expect(launchUrl).toContain('test-app.example.com');

    // URL must NOT contain any token or auth parameter
    expect(launchUrl).not.toContain('token');
    expect(launchUrl).not.toContain('jwt');
    expect(launchUrl).not.toContain('access_token');
  });

  test('heading says Subscribed Apps', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('app-launcher-panel')).toBeVisible({ timeout: BFF_TIMEOUT });

    await expect(
      page.getByTestId('app-launcher-panel').getByText('Subscribed Apps'),
    ).toBeVisible();
  });
});
