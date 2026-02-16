#!/usr/bin/env node
import fs from "node:fs";
import { chromium } from "playwright";

/**
 * Initialize ChatGPT browser storage state
 *
 * This script:
 * 1. Launches a browser
 * 2. Navigates to ChatGPT
 * 3. Waits for you to log in manually
 * 4. Saves the authentication state to .browser-profiles/chatgpt-state.json
 *
 * Usage:
 *   node scripts/init-chatgpt-storage-state.mjs
 */

const STORAGE_DIR = ".browser-profiles";
const STORAGE_STATE_PATH = `${STORAGE_DIR}/chatgpt-state.json`;

console.log("=== ChatGPT Storage State Initialization ===");
console.log("");
console.log("This will:");
console.log("1. Open a browser to ChatGPT");
console.log("2. Wait for you to log in");
console.log("3. Save the session to " + STORAGE_STATE_PATH);
console.log("");
console.log("Please log in to ChatGPT when the browser opens.");
console.log("Once you're logged in and see the ChatGPT interface,");
console.log("press Enter in this terminal to save the session.");
console.log("");

// Create storage directory if it doesn't exist
if (!fs.existsSync(STORAGE_DIR)) {
  fs.mkdirSync(STORAGE_DIR, { recursive: true });
  console.log(`✓ Created directory: ${STORAGE_DIR}`);
}

// Launch browser
console.log("Launching browser...");
const browser = await chromium.launch({
  headless: false,
  args: [
    '--disable-blink-features=AutomationControlled',
    '--no-sandbox'
  ]
});

const context = await browser.newContext({
  viewport: { width: 1280, height: 800 },
  userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36'
});

const page = await context.newPage();

// Navigate to ChatGPT
console.log("Navigating to ChatGPT...");
await page.goto("https://chatgpt.com", { waitUntil: "domcontentloaded" });

console.log("");
console.log("✓ Browser opened to ChatGPT");
console.log("");
console.log("Please complete the following steps:");
console.log("1. Log in to ChatGPT if you're not already logged in");
console.log("2. Wait until you see the ChatGPT chat interface");
console.log("");
console.log("Waiting for login (checking every 5 seconds)...");
console.log("The script will automatically save the session once you're logged in.");
console.log("(Ctrl+C to cancel)");
console.log("");

// Wait for authentication by checking for ChatGPT-specific elements
let isLoggedIn = false;
while (!isLoggedIn) {
  try {
    // Check for elements that indicate the user is logged in
    // Look for the chat interface or user menu
    const hasChat = await page.locator('[data-testid="conversation-turn-0"]').count() > 0;
    const hasPrompt = await page.locator('textarea[placeholder*="Message"]').count() > 0;
    const hasUserMenu = await page.locator('[data-headlessui-state]').count() > 0;

    if (hasChat || hasPrompt || hasUserMenu) {
      isLoggedIn = true;
      console.log("✓ Detected ChatGPT interface - you appear to be logged in!");
    } else {
      // Check if we're on a login page
      const url = page.url();
      if (url.includes('chatgpt.com') && !url.includes('auth')) {
        // We're on ChatGPT but not on auth page - might be logged in
        // Wait a bit more to be sure
        await page.waitForTimeout(2000);
        const hasPromptNow = await page.locator('textarea').count() > 0;
        if (hasPromptNow) {
          isLoggedIn = true;
          console.log("✓ Detected ChatGPT interface!");
        }
      }

      if (!isLoggedIn) {
        await page.waitForTimeout(5000);
        process.stdout.write(".");
      }
    }
  } catch (e) {
    // Keep waiting
    await page.waitForTimeout(5000);
    process.stdout.write(".");
  }
}

console.log("");
console.log("");
console.log("Saving session state...");
await context.storageState({ path: STORAGE_STATE_PATH });

console.log(`✓ Session saved to: ${STORAGE_STATE_PATH}`);
console.log("");

// Close browser
await browser.close();

console.log("✓ Browser closed");
console.log("");
console.log("Setup complete! You can now use the ChatGPT worker scripts.");
console.log("");
