#!/usr/bin/env node
import { chromium } from "playwright";

/**
 * Open an authenticated browser to ChatGPT
 * Uses the saved storage state from .browser-profiles/chatgpt-state.json
 */

const storageStatePath = ".browser-profiles/chatgpt-state.json";

console.log("Opening authenticated browser to ChatGPT...");
console.log("Press Ctrl+C to close the browser.");
console.log("");

const browser = await chromium.launch({
  headless: false,
  args: [
    '--disable-blink-features=AutomationControlled',
    '--no-sandbox'
  ]
});

const context = await browser.newContext({
  storageState: storageStatePath,
  viewport: { width: 1280, height: 800 },
  userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36'
});

const page = await context.newPage();
await page.goto("https://chatgpt.com", { waitUntil: "domcontentloaded" });

console.log("✓ Browser opened");
console.log("Current URL:", page.url());
console.log("");
console.log("You can now interact with ChatGPT manually.");
console.log("When you're done, copy the conversation URL and update .flywheel/chatgpt.json");
console.log("");

// Keep browser open
process.on('SIGINT', async () => {
  console.log("\nClosing browser...");
  await browser.close();
  process.exit(0);
});

// Keep process alive
await new Promise(() => {});
