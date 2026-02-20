// ============================================================
// Playwright configuration
// Smoke tests run against the live identity-auth service.
// ============================================================
import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  fullyParallel: false, // serial to avoid lock contention on auth state
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  reporter: 'list',

  use: {
    baseURL: process.env.BASE_URL ?? 'http://localhost:3000',
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },

  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],

  // Start the Next.js server automatically if not already running.
  // Uses production build (npm run start) for faster startup; falls back to dev.
  // CI skips auto-start — assumes server is already running.
  webServer: process.env.CI
    ? undefined
    : {
        command: process.env.PW_USE_DEV ? 'npm run dev' : 'npm run start',
        url: 'http://localhost:3000',
        reuseExistingServer: true,
        timeout: 120_000,
      },
});
