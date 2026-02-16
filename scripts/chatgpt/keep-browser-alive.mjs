#!/usr/bin/env node
import fs from "node:fs";
import { execSync } from "node:child_process";
import { chromium } from "playwright";

/**
 * Keep a persistent browser alive for ChatGPT interactions
 *
 * This script:
 * 1. Launches a browser with authentication
 * 2. Hides the window immediately
 * 3. Navigates to ChatGPT conversation
 * 4. Keeps the browser alive
 * 5. Writes endpoint for post-and-extract.mjs to connect to
 */

function usage(exitCode = 1) {
  console.error(`
keep-browser-alive.mjs

Keep a persistent browser alive for ChatGPT interactions.

Usage:
  node scripts/chatgpt/keep-browser-alive.mjs \\
    --conversation-url https://chatgpt.com/c/... \\
    --endpoint-file .flywheel/browser-endpoint.txt \\
    --pid-file .flywheel/browser.pid
`);
  process.exit(exitCode);
}

function parseArgs(argv) {
  const args = {};
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith("--")) {
      const key = a.slice(2);
      const next = argv[i + 1];
      if (!next || next.startsWith("--")) {
        args[key] = true;
      } else {
        args[key] = next;
        i++;
      }
    }
  }
  return args;
}

async function main() {
  const args = parseArgs(process.argv);

  if (args.help || args.h) usage(0);

  const conversationUrl = args["conversation-url"];
  const endpointFile = args["endpoint-file"] || ".flywheel/browser-endpoint.txt";
  const pidFile = args["pid-file"] || ".flywheel/browser.pid";

  if (!conversationUrl) {
    console.error("Missing required --conversation-url");
    usage(1);
  }

  // Check for storage state
  const storageStatePath = ".browser-profiles/chatgpt-state.json";
  if (!fs.existsSync(storageStatePath)) {
    console.error(`Storage state not found: ${storageStatePath}`);
    console.error("Run: node scripts/init-chatgpt-storage-state.mjs");
    process.exit(1);
  }

  console.error("=== Starting Persistent Browser ===");
  console.error(`Conversation: ${conversationUrl}`);
  console.error("");

  // Launch browser server
  const browserServer = await chromium.launchServer({
    headless: false,
    args: [
      '--disable-blink-features=AutomationControlled',
      '--no-sandbox',
      '--disable-setuid-sandbox',
      '--disable-dev-shm-usage',
      '--disable-web-security'
    ]
  });

  const wsEndpoint = browserServer.wsEndpoint();
  console.error(`WebSocket endpoint: ${wsEndpoint}`);

  // Connect to the server to set up initial page
  const browser = await chromium.connect(wsEndpoint);

  // Navigate first, THEN hide (ChatGPT needs to load while visible)
  // We'll hide after the page is ready

  // Create authenticated context
  const context = await browser.newContext({
    storageState: storageStatePath,
    viewport: { width: 1280, height: 800 },
    userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36'
  });

  const page = await context.newPage();

  // Navigate to conversation
  console.error(`Navigating to: ${conversationUrl}`);
  await page.goto(conversationUrl, { waitUntil: "domcontentloaded", timeout: 30000 });
  console.error("✓ Page loaded");

  // Wait for ChatGPT to load
  await page.waitForTimeout(2000);

  // NOW hide the browser window (after page loaded)
  try {
    execSync('osascript -e \'tell application "System Events" to set visible of process "Chromium" to false\'', { timeout: 2000 });
    console.error("✓ Browser window hidden");
  } catch (e) {
    try {
      execSync('osascript -e \'tell application "System Events" to set visible of process "Google Chrome" to false\'', { timeout: 2000 });
      console.error("✓ Browser window hidden");
    } catch (e2) {
      console.error("⚠️  Could not hide browser window");
    }
  }

  // Write endpoint file
  fs.writeFileSync(endpointFile, wsEndpoint + "\n", "utf8");
  console.error(`✓ Endpoint written to: ${endpointFile}`);

  // Write PID file
  fs.writeFileSync(pidFile, process.pid.toString() + "\n", "utf8");
  console.error(`✓ PID ${process.pid} written to: ${pidFile}`);

  console.error("");
  console.error("Browser is ready. Press Ctrl+C to stop.");
  console.error("");

  // Handle cleanup
  let isShuttingDown = false;
  async function cleanup() {
    if (isShuttingDown) return;
    isShuttingDown = true;

    console.error("");
    console.error("Shutting down...");

    try {
      await page.close().catch(() => {});
      await context.close().catch(() => {});
      await browser.close().catch(() => {});
      await browserServer.close().catch(() => {});
    } catch (e) {
      // Ignore cleanup errors
    }

    try {
      if (fs.existsSync(pidFile)) fs.unlinkSync(pidFile);
      if (fs.existsSync(endpointFile)) fs.unlinkSync(endpointFile);
    } catch (e) {
      // Ignore file cleanup errors
    }

    console.error("✓ Shutdown complete");
    process.exit(0);
  }

  process.on('SIGINT', cleanup);
  process.on('SIGTERM', cleanup);

  // Health check every 60 seconds
  setInterval(async () => {
    try {
      await page.evaluate(() => document.title);
      console.error(`[${new Date().toISOString()}] Health check: OK`);
    } catch (e) {
      console.error(`[${new Date().toISOString()}] Health check failed: ${e.message}`);
    }
  }, 60000);

  // Keep process alive
  await new Promise(() => {});
}

main().catch((err) => {
  console.error(`ERROR: ${err.message}`);
  process.exit(1);
});
