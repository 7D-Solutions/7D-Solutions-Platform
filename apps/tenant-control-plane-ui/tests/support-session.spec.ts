// ============================================================
// Support Session E2E — start/stop, reason required, indicator
// Verifies: Support session start requires reason, actor_type
// becomes support, banner/indicator shown, end returns to staff.
// Verification: npx playwright test -g "Support Session"
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

const BFF_TIMEOUT = 15000;

test.describe('Support Session', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    // Clear persisted view state
    await page.request.delete('/api/preferences/view-tenant-detail-home').catch(() => {});

    // Ensure no leftover support session — end any active one
    await page.request.post('/api/tenants/test-tenant-001/support-sessions/end').catch(() => {});
  });

  test('Access tab shows support session section', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    await expect(page.getByTestId('support-session-section')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByText('Support Sessions')).toBeVisible();
  });

  test('start button opens modal requiring reason', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    await expect(page.getByTestId('start-support-session-btn')).toBeVisible({ timeout: BFF_TIMEOUT });
    await page.getByTestId('start-support-session-btn').click();

    // Modal opens
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });
    await expect(page.getByRole('heading', { name: 'Start Support Session' })).toBeVisible();

    // Confirm button disabled without reason
    await expect(page.getByTestId('confirm-start-session-btn')).toBeDisabled();

    // Type a reason
    await page.getByTestId('support-reason-input').fill('Customer reported billing issue');
    await expect(page.getByTestId('confirm-start-session-btn')).toBeEnabled();
  });

  test('cancel closes start session modal', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    await page.getByTestId('start-support-session-btn').click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });

    await page.getByRole('button', { name: 'Cancel' }).click();
    await expect(page.getByRole('dialog')).not.toBeVisible({ timeout: 5000 });
  });

  test('starting session calls BFF and sets support actor_type', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    await page.getByTestId('start-support-session-btn').click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });

    await page.getByTestId('support-reason-input').fill('Investigating access issue');

    // Click confirm and wait for BFF response
    const [startRes] = await Promise.all([
      page.waitForResponse(
        (res) =>
          res.url().includes('/support-sessions/start') &&
          res.request().method() === 'POST',
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('confirm-start-session-btn').click(),
    ]);

    expect(startRes.status()).toBe(200);
    const body = await startRes.json();
    expect(body.actor_type).toBe('support');

    // Modal should close
    await expect(page.getByRole('dialog')).not.toBeVisible({ timeout: 5000 });

    // Verify actor_type is support via /api/auth/me
    const meRes = await page.request.get('/api/auth/me');
    expect(meRes.ok()).toBeTruthy();
    const me = await meRes.json();
    expect(me.actor_type).toBe('support');
  });

  test('active support session shows banner indicator', async ({ page }) => {
    // Start a session first via API
    const startRes = await page.request.post(
      '/api/tenants/test-tenant-001/support-sessions/start',
      {
        data: { reason: 'Testing banner display' },
        headers: { 'Content-Type': 'application/json' },
      },
    );
    expect(startRes.ok()).toBeTruthy();

    // Navigate to a page and check for banner
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('support-session-banner')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByText('Support session active')).toBeVisible();
    await expect(page.getByTestId('banner-end-session-btn')).toBeVisible();
  });

  test('active session shows in Access tab as active', async ({ page }) => {
    // Start session via API
    await page.request.post(
      '/api/tenants/test-tenant-001/support-sessions/start',
      {
        data: { reason: 'Testing active display' },
        headers: { 'Content-Type': 'application/json' },
      },
    );

    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    // The support session section should show active state
    await expect(page.getByTestId('support-session-section')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('end-support-session-btn')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByText('Support session active for this tenant')).toBeVisible();
  });

  test('end session via banner returns to staff actor_type', async ({ page }) => {
    // Start session via API
    await page.request.post(
      '/api/tenants/test-tenant-001/support-sessions/start',
      {
        data: { reason: 'Testing end via banner' },
        headers: { 'Content-Type': 'application/json' },
      },
    );

    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('support-session-banner')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Click end session on banner
    const [endRes] = await Promise.all([
      page.waitForResponse(
        (res) =>
          res.url().includes('/support-sessions/end') &&
          res.request().method() === 'POST',
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('banner-end-session-btn').click(),
    ]);

    expect(endRes.status()).toBe(200);

    // Banner should disappear
    await expect(page.getByTestId('support-session-banner')).not.toBeVisible({ timeout: BFF_TIMEOUT });

    // Verify actor_type is staff via /api/auth/me
    const meRes = await page.request.get('/api/auth/me');
    expect(meRes.ok()).toBeTruthy();
    const me = await meRes.json();
    expect(me.actor_type).toBe('staff');
  });

  test('end session via Access tab end button', async ({ page }) => {
    // Start session via API
    await page.request.post(
      '/api/tenants/test-tenant-001/support-sessions/start',
      {
        data: { reason: 'Testing end via Access tab' },
        headers: { 'Content-Type': 'application/json' },
      },
    );

    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('end-support-session-btn')).toBeVisible({ timeout: BFF_TIMEOUT });

    // End session from the Access tab
    const [endRes] = await Promise.all([
      page.waitForResponse(
        (res) =>
          res.url().includes('/support-sessions/end') &&
          res.request().method() === 'POST',
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('end-support-session-btn').click(),
    ]);

    expect(endRes.status()).toBe(200);

    // Should revert to showing start button
    await expect(page.getByTestId('start-support-session-btn')).toBeVisible({ timeout: BFF_TIMEOUT });
  });

  test('full flow: start -> verify active -> end -> verify staff', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    // 1. Start support session
    await page.getByTestId('start-support-session-btn').click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });
    await page.getByTestId('support-reason-input').fill('Full flow E2E test');

    await Promise.all([
      page.waitForResponse(
        (res) => res.url().includes('/support-sessions/start'),
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('confirm-start-session-btn').click(),
    ]);

    await expect(page.getByRole('dialog')).not.toBeVisible({ timeout: 5000 });

    // 2. Verify actor_type is support via direct API
    const meRes1 = await page.request.get('/api/auth/me');
    const me1 = await meRes1.json();
    expect(me1.actor_type).toBe('support');

    // 3. Reload page to pick up support session in layout meQuery
    await page.reload();
    await expect(page.getByTestId('support-session-banner')).toBeVisible({ timeout: BFF_TIMEOUT });

    // 4. End session via banner
    await Promise.all([
      page.waitForResponse(
        (res) => res.url().includes('/support-sessions/end'),
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('banner-end-session-btn').click(),
    ]);

    // 5. Verify banner gone
    await expect(page.getByTestId('support-session-banner')).not.toBeVisible({ timeout: BFF_TIMEOUT });

    // 6. Verify actor_type is staff
    const meRes2 = await page.request.get('/api/auth/me');
    const me2 = await meRes2.json();
    expect(me2.actor_type).toBe('staff');
  });

  test('start session without reason is rejected (direct API)', async ({ page }) => {
    const res = await page.request.post(
      '/api/tenants/test-tenant-001/support-sessions/start',
      {
        data: { reason: '' },
        headers: { 'Content-Type': 'application/json' },
      },
    );
    // Route validates empty reason and returns 400
    expect(res.ok()).toBeFalsy();
    expect(res.status()).toBe(400);
  });

  test('end session without active session returns error', async ({ page }) => {
    // Ensure no active session — double-end to guarantee clean state
    await page.request.post('/api/tenants/test-tenant-001/support-sessions/end').catch(() => {});

    const res = await page.request.post(
      '/api/tenants/test-tenant-001/support-sessions/end',
    );
    // Route checks for support cookie and returns 400 if missing
    expect(res.ok()).toBeFalsy();
    expect(res.status()).toBe(400);
  });
});
