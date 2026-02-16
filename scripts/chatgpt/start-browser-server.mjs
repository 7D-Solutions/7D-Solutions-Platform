#!/usr/bin/env node
import fs from "node:fs";
import { chromium } from "playwright";

/**
 * Start a persistent Playwright browser server
 *
 * This launches a browser in server mode and outputs the WebSocket endpoint
 * that clients can connect to. This allows multiple post-and-extract.mjs
 * calls to reuse the same browser session (no window spam, no focus stealing).
 *
 * Usage:
 *   node scripts/chatgpt/start-browser-server.mjs \
 *     --pid-file .flywheel/browser-server.pid \
 *     --endpoint-file .flywheel/browser-endpoint.txt
 */

function usage(exitCode = 1) {
  console.error(`
start-browser-server.mjs

Start a persistent Playwright browser server for ChatGPT integration.

Usage:
  node scripts/chatgpt/start-browser-server.mjs \\
    [--pid-file .flywheel/browser-server.pid] \\
    [--endpoint-file .flywheel/browser-endpoint.txt] \\
    [--storage-state .browser-profiles/chatgpt-state.json]

The browser will stay open and the process will keep running.
Kill the process or send SIGTERM to stop the browser.
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

async function launchBrowser() {
  return await chromium.launchServer({
    headless: false,  // Use headed mode to avoid detection
    args: [
      '--disable-blink-features=AutomationControlled',
      '--no-sandbox',
      '--disable-setuid-sandbox',
      '--disable-dev-shm-usage',
      '--disable-web-security'
    ]
  });
}

async function startBrowserServer(storageStatePath, pidFile, endpointFile) {
  console.error("=== Starting Browser Server ===");
  console.error(`Storage state: ${storageStatePath}`);
  console.error(`PID file: ${pidFile}`);
  console.error(`Endpoint file: ${endpointFile}`);
  console.error("");

  let browserServer = null;
  let healthCheckInterval = null;
  let isShuttingDown = false;

  const startBrowser = async () => {
    try {
      // Launch browser in server mode
      browserServer = await launchBrowser();

      const wsEndpoint = browserServer.wsEndpoint();
      console.error(`✓ Browser server started`);
      console.error(`WebSocket endpoint: ${wsEndpoint}`);
      console.error("");

      // Write PID file
      const pid = process.pid;
      fs.writeFileSync(pidFile, `${pid}\n`, "utf8");
      console.error(`✓ PID ${pid} written to: ${pidFile}`);

      // Write endpoint file
      fs.writeFileSync(endpointFile, `${wsEndpoint}\n`, "utf8");
      console.error(`✓ Endpoint written to: ${endpointFile}`);
      console.error("");

      // Health check: connect and verify
      console.error("Running initial health check...");
      const browser = await chromium.connect(wsEndpoint);
      const version = await browser.version();
      console.error(`✓ Health check passed (browser version: ${version})`);
      await browser.close();  // Close connection, not the server
      console.error("");

      // Monitor browser disconnect
      browserServer.on('close', async () => {
        if (isShuttingDown) {
          console.error("Browser closed during shutdown (expected)");
          return;
        }

        console.error("");
        console.error("⚠️  Browser server disconnected unexpectedly!");
        console.error("Attempting to restart...");
        console.error("");

        // Clear health check interval
        if (healthCheckInterval) {
          clearInterval(healthCheckInterval);
          healthCheckInterval = null;
        }

        // Wait a bit before restarting
        await new Promise(resolve => setTimeout(resolve, 2000));

        // Restart browser
        await startBrowser();
      });

      // Periodic health check (every 60 seconds)
      healthCheckInterval = setInterval(async () => {
        try {
          const browser = await chromium.connect(wsEndpoint);
          await browser.version();  // Quick health check
          await browser.close();
          console.error(`[${new Date().toISOString()}] Health check: OK`);
        } catch (e) {
          console.error(`[${new Date().toISOString()}] Health check failed: ${e.message}`);
          // Don't restart here - let the 'close' event handler do it
        }
      }, 60000);

      console.error("Browser server is running. Press Ctrl+C to stop.");
      console.error("");

    } catch (error) {
      console.error(`Failed to start browser: ${error.message}`);
      throw error;
    }
  };

  // Cleanup handler
  const cleanup = async () => {
    if (isShuttingDown) return;
    isShuttingDown = true;

    console.error("");
    console.error("Shutting down browser server...");

    // Clear health check
    if (healthCheckInterval) {
      clearInterval(healthCheckInterval);
    }

    // Remove files
    if (fs.existsSync(pidFile)) {
      fs.unlinkSync(pidFile);
      console.error(`✓ Removed PID file: ${pidFile}`);
    }
    if (fs.existsSync(endpointFile)) {
      fs.unlinkSync(endpointFile);
      console.error(`✓ Removed endpoint file: ${endpointFile}`);
    }

    // Close browser
    if (browserServer) {
      await browserServer.close();
      console.error("✓ Browser server stopped");
    }

    process.exit(0);
  };

  process.on('SIGTERM', cleanup);
  process.on('SIGINT', cleanup);

  // Start the browser
  await startBrowser();

  // Keep process alive
  setInterval(() => {}, 1000);
}

(async function main() {
  const args = parseArgs(process.argv);

  if (args.help || args.h) usage(0);

  const pidFile = args["pid-file"] || ".flywheel/browser-server.pid";
  const endpointFile = args["endpoint-file"] || ".flywheel/browser-endpoint.txt";
  const storageStatePath = args["storage-state"] || ".browser-profiles/chatgpt-state.json";

  // Check for storage state
  if (!fs.existsSync(storageStatePath)) {
    console.error(`Storage state not found: ${storageStatePath}`);
    console.error("Run: node scripts/init-chatgpt-storage-state.mjs");
    process.exit(1);
  }

  // Ensure directories exist
  const pidDir = pidFile.substring(0, pidFile.lastIndexOf("/"));
  if (pidDir && !fs.existsSync(pidDir)) {
    fs.mkdirSync(pidDir, { recursive: true });
  }
  const endpointDir = endpointFile.substring(0, endpointFile.lastIndexOf("/"));
  if (endpointDir && !fs.existsSync(endpointDir)) {
    fs.mkdirSync(endpointDir, { recursive: true });
  }

  await startBrowserServer(storageStatePath, pidFile, endpointFile);
})().catch((err) => {
  console.error(`ERROR: ${err.message}`);
  process.exit(1);
});
