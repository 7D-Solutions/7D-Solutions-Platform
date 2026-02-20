// ============================================================
// Notifications E2E — bell icon, panel, mark-read, empty state
// Verifies: bell renders, panel opens on click, empty state shown
// when no backend notifications exist, mark-all-read works.
// Requires: Next.js dev server running, auth fixture available.
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('Notifications', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    await page.goto('/tenants');
    // Wait for shell to render
    await expect(page.getByTestId('notification-bell')).toBeVisible({ timeout: 10000 });
  });

  test('bell icon is visible in the top bar', async ({ page }) => {
    const bell = page.getByTestId('notification-bell');
    await expect(bell).toBeVisible();
    await expect(bell).toHaveAttribute(
      'aria-label',
      /Notifications/,
    );
  });

  test('clicking bell opens notification panel', async ({ page }) => {
    await page.getByTestId('notification-bell').click();
    const panel = page.getByTestId('notification-panel');
    await expect(panel).toBeVisible();
    // Header should show "Notifications"
    await expect(panel.getByRole('heading', { name: 'Notifications' })).toBeVisible();
  });

  test('empty state shown when no notifications', async ({ page }) => {
    await page.getByTestId('notification-bell').click();
    const panel = page.getByTestId('notification-panel');
    await expect(panel).toBeVisible();
    await expect(panel.getByTestId('notification-empty')).toBeVisible();
    await expect(panel.getByText('No notifications')).toBeVisible();
  });

  test('clicking outside closes the panel', async ({ page }) => {
    await page.getByTestId('notification-bell').click();
    await expect(page.getByTestId('notification-panel')).toBeVisible();
    // Click outside the panel
    await page.locator('main').click();
    await expect(page.getByTestId('notification-panel')).not.toBeVisible();
  });

  test('no unread badge when notifications are empty', async ({ page }) => {
    // Badge should not be present when there are no unread notifications
    await expect(page.getByTestId('notification-badge')).not.toBeVisible();
  });

  test('BFF /api/notifications returns deterministic empty state', async ({ page }) => {
    const res = await page.request.get('/api/notifications');
    expect(res.ok()).toBeTruthy();
    const data = await res.json();
    expect(data).toHaveProperty('notifications');
    expect(data).toHaveProperty('unread_count');
    expect(Array.isArray(data.notifications)).toBeTruthy();
    expect(data.unread_count).toBe(0);
  });

  test('BFF /api/notifications/mark-read returns success', async ({ page }) => {
    const res = await page.request.post('/api/notifications/mark-read', {
      data: { all: true },
      headers: { 'Content-Type': 'application/json' },
    });
    expect(res.ok()).toBeTruthy();
    const data = await res.json();
    expect(data).toHaveProperty('success', true);
  });
});
