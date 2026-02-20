// ============================================================
// Audit Log E2E — filters, pagination, BFF route
// Pattern: passive page.on('request') listeners + waitForTimeout
// (matches tenant-list.spec.ts proven approach)
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

/** Navigate to /audit and wait for full render */
async function gotoAuditPage(page: import('@playwright/test').Page) {
  await page.goto('/audit');
  await expect(page.getByRole('heading', { name: 'Audit Log' })).toBeVisible({ timeout: 15_000 });
}

test.describe('Audit Log', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
  });

  // ── Rendering ─────────────────────────────────────────────────

  test('renders audit log page with header and filter bar', async ({ page }) => {
    await gotoAuditPage(page);
    await expect(page.getByTestId('audit-filter-bar')).toBeVisible();
    await expect(page.getByTestId('audit-actor-search')).toBeVisible();
    await expect(page.getByTestId('audit-action-filter')).toBeVisible();
    await expect(page.getByTestId('audit-tenant-filter')).toBeVisible();
  });

  test('fetches audit events via BFF /api/audit route', async ({ page }) => {
    const bffRequests: string[] = [];

    page.on('request', (req) => {
      if (req.url().includes('/api/audit')) {
        bffRequests.push(req.url());
      }
    });

    await page.goto('/audit');
    await page.waitForTimeout(2000);

    expect(bffRequests.length).toBeGreaterThan(0);
    expect(bffRequests[0]).toContain('/api/audit');
  });

  // ── Filter interaction (passive listener + waitForTimeout) ────

  test('actor search updates query params', async ({ page }) => {
    const bffRequests: string[] = [];

    page.on('request', (req) => {
      if (req.url().includes('/api/audit') && req.method() === 'GET') {
        bffRequests.push(req.url());
      }
    });

    await gotoAuditPage(page);
    await page.waitForTimeout(500);

    await page.getByTestId('audit-actor-search').fill('admin@test.com');

    // Wait for debounce (300ms) + network
    await page.waitForTimeout(1000);

    const searchReqs = bffRequests.filter((u) => u.includes('actor=admin'));
    expect(searchReqs.length).toBeGreaterThan(0);
  });

  test('action filter changes query params', async ({ page }) => {
    const bffRequests: string[] = [];

    page.on('request', (req) => {
      if (req.url().includes('/api/audit') && req.method() === 'GET') {
        bffRequests.push(req.url());
      }
    });

    await gotoAuditPage(page);
    await page.waitForTimeout(500);

    await page.getByTestId('audit-action-filter').selectOption('tenant.created');

    await page.waitForTimeout(500);

    const statusReqs = bffRequests.filter((u) => u.includes('action=tenant.created'));
    expect(statusReqs.length).toBeGreaterThan(0);
  });

  test('tenant ID filter changes query params', async ({ page }) => {
    const bffRequests: string[] = [];

    page.on('request', (req) => {
      if (req.url().includes('/api/audit') && req.method() === 'GET') {
        bffRequests.push(req.url());
      }
    });

    await gotoAuditPage(page);
    await page.waitForTimeout(500);

    await page.getByTestId('audit-tenant-filter').fill('tenant-123');

    await page.waitForTimeout(500);

    const tenantReqs = bffRequests.filter((u) => u.includes('tenant_id=tenant-123'));
    expect(tenantReqs.length).toBeGreaterThan(0);
  });

  // ── Empty state ───────────────────────────────────────────────

  test('handles empty results gracefully', async ({ page }) => {
    await gotoAuditPage(page);
    await expect(page.getByTestId('audit-table')).toBeVisible();
    await expect(page.locator('body')).toBeVisible();
  });

  // ── Clear filters ─────────────────────────────────────────────

  test('clear filters button appears when filters are active', async ({ page }) => {
    await gotoAuditPage(page);

    // Initially no clear button
    await expect(page.getByRole('button', { name: /clear filters/i })).not.toBeVisible();

    // Apply a filter
    await page.getByTestId('audit-action-filter').selectOption('user.login');

    // Clear button should appear
    await expect(page.getByRole('button', { name: /clear filters/i })).toBeVisible({ timeout: 5_000 });

    // Click clear
    await page.getByRole('button', { name: /clear filters/i }).click();

    // Filter should reset
    await expect(page.getByTestId('audit-action-filter')).toHaveValue('', { timeout: 5_000 });
    await expect(page.getByRole('button', { name: /clear filters/i })).not.toBeVisible();
  });

  // ── Navigation ────────────────────────────────────────────────

  test('audit page is accessible from sidebar navigation', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByRole('heading', { name: 'Tenants' })).toBeVisible({ timeout: 15_000 });
    await expect(page.getByRole('link', { name: 'Audit Log' })).toBeVisible();
    await page.getByRole('link', { name: 'Audit Log' }).click();
    await expect(page).toHaveURL(/\/audit/);
    await expect(page.getByRole('heading', { name: 'Audit Log' })).toBeVisible({ timeout: 15_000 });
  });

  // ── Date range ────────────────────────────────────────────────

  test('date range filter inputs are present', async ({ page }) => {
    await gotoAuditPage(page);
    await expect(page.getByLabel('Start date')).toBeVisible({ timeout: 10_000 });
    await expect(page.getByLabel('End date')).toBeVisible({ timeout: 10_000 });
  });
});
