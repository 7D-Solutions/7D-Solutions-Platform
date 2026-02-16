#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { execSync } from "node:child_process";
import { chromium } from "playwright";

/**
 * Browser Worker - keeps ONE browser open and processes messages
 *
 * This solves the window popup problem by:
 * 1. Opening browser ONCE and hiding it immediately
 * 2. Keeping it alive and processing messages in a loop
 * 3. Never closing the browser until the process exits
 */

const STORAGE_STATE = ".browser-profiles/chatgpt-state.json";
const REQUEST_FILE = ".flywheel/browser-request.json";
const RESPONSE_FILE = ".flywheel/browser-response.json";
const READY_FILE = ".flywheel/browser-ready.txt";

console.error("=== Browser Worker Starting ===");

// Launch browser ONCE
const browser = await chromium.launch({
  headless: false,
  args: [
    '--disable-blink-features=AutomationControlled',
    '--no-sandbox',
    '--disable-setuid-sandbox',
    '--disable-dev-shm-usage',
    '--disable-web-security',
    '--window-position=3000,3000',
    '--window-size=1,1'
  ]
});

// Hide immediately
setTimeout(() => {
  try {
    execSync('osascript -e \'tell application "System Events" to set visible of process "Chromium" to false\'', { timeout: 1000 });
  } catch (e) {
    try {
      execSync('osascript -e \'tell application "System Events" to set visible of process "Google Chrome" to false\'', { timeout: 1000 });
    } catch (e2) {}
  }
}, 300);

const context = await browser.newContext({
  storageState: STORAGE_STATE,
  viewport: { width: 1280, height: 800 },
  userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36'
});

const page = await context.newPage();

console.error("✓ Browser opened and hidden");
console.error("");
console.error("Worker ready. Watching for requests at:", REQUEST_FILE);
console.error("Press Ctrl+C to stop.");
console.error("");

// Write ready signal
fs.writeFileSync(READY_FILE, Date.now().toString() + "\n");

// Process loop
while (true) {
  // Check for request file
  if (fs.existsSync(REQUEST_FILE)) {
    try {
      const request = JSON.parse(fs.readFileSync(REQUEST_FILE, "utf8"));

      // Delete request file immediately to prevent reprocessing
      fs.unlinkSync(REQUEST_FILE);

      console.error(`[${new Date().toISOString()}] Processing request: ${request.conversation_url}`);

      // Navigate to conversation
      await page.goto(request.conversation_url, { waitUntil: "domcontentloaded", timeout: 30000 });

      // Wait for input
      const input = page.locator('[contenteditable="true"]').last();
      await input.waitFor({ state: "visible", timeout: 10000 });
      await page.waitForTimeout(1000);

      // Capture baseline
      const baselineCount = await page.locator('[data-message-author-role="assistant"]').count();

      // Post message
      await input.evaluate((el, text) => {
        el.textContent = text;
        el.dispatchEvent(new Event("input", { bubbles: true }));
      }, request.message);

      await page.waitForTimeout(1000);
      await input.press('Enter');

      // Wait for response
      const messageLocator = page.locator('[data-message-author-role="assistant"]');
      let newMessageAppeared = false;
      const waitStart = Date.now();

      while (!newMessageAppeared && (Date.now() - waitStart < 10000)) {
        const currentCount = await messageLocator.count();
        if (currentCount > baselineCount) {
          newMessageAppeared = true;
        } else {
          await page.waitForTimeout(500);
        }
      }

      if (!newMessageAppeared) {
        throw new Error("Timeout waiting for response");
      }

      // Wait for completion
      const stopButton = page.locator('button').filter({ hasText: /stop generating/i });
      let stopButtonGone = false;
      const startTime = Date.now();

      while (!stopButtonGone && (Date.now() - startTime < 120000)) {
        await page.waitForTimeout(3000);
        const isGenerating = await stopButton.isVisible().catch(() => false);
        if (!isGenerating) {
          stopButtonGone = true;
        }
      }

      // Wait for stability
      await page.waitForTimeout(5000);
      const lastMessage = page.locator('[data-message-author-role="assistant"]').last();
      const rawText = await lastMessage.innerText().catch(() => "");

      // Extract JSON if present
      let extracted_json = null;
      let parse_ok = false;

      // Try 1: Look for ```json fences
      const jsonMatch = rawText.match(/```json\s*([\s\S]*?)\s*```/i);
      if (jsonMatch) {
        try {
          extracted_json = JSON.parse(jsonMatch[1]);
          parse_ok = true;
        } catch (e) {}
      }

      // Try 2: Look for code blocks in DOM (ChatGPT renders JSON in <pre><code>)
      if (!parse_ok) {
        try {
          const codeBlocks = await lastMessage.locator('pre code').all();
          if (codeBlocks.length > 0) {
            // Try the last code block first (most likely to be the answer)
            for (let i = codeBlocks.length - 1; i >= 0; i--) {
              const codeText = await codeBlocks[i].innerText();
              try {
                extracted_json = JSON.parse(codeText);
                parse_ok = true;
                break;
              } catch (e) {
                // Not valid JSON, try next
              }
            }
          }
        } catch (e) {}
      }

      // Try 3: Look for "JSON\n{" pattern (ChatGPT's fallback format)
      if (!parse_ok) {
        const simpleMatch = rawText.match(/^JSON\s*\n\s*(\{[\s\S]*\}|\[[\s\S]*\])\s*$/im);
        if (simpleMatch) {
          try {
            extracted_json = JSON.parse(simpleMatch[1]);
            parse_ok = true;
          } catch (e) {}
        }
      }

      // Write response
      const response = {
        ok: true,
        raw_text: rawText,
        parse_ok: parse_ok,
        extracted_json: extracted_json,
        error: parse_ok ? null : "NO_JSON_STRUCTURE_FOUND"
      };

      fs.writeFileSync(RESPONSE_FILE, JSON.stringify(response, null, 2) + "\n");
      console.error(`✓ Response written (${rawText.length} chars)`);

    } catch (err) {
      const errorResponse = {
        ok: false,
        raw_text: "",
        parse_ok: false,
        extracted_json: null,
        error: err.message
      };
      fs.writeFileSync(RESPONSE_FILE, JSON.stringify(errorResponse, null, 2) + "\n");
      console.error(`✗ Error: ${err.message}`);
    }
  }

  // Sleep before next check
  await page.waitForTimeout(500);
}
