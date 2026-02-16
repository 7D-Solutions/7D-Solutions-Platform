#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { chromium } from "playwright";

/**
 * Ask a question to Grok and extract response
 *
 * Uses Playwright DIRECTLY (not MCP) to:
 * - Load authenticated Grok session
 * - Post question
 * - Wait for response
 * - Extract response text
 * - Return structured JSON
 */

function usage(exitCode = 1) {
  console.error(`
ask-grok.mjs

Ask a question to Grok and extract the response.

Usage:
  node scripts/ask-grok.mjs \\
    --question "What is tmux?" \\
    --out tmp/grok-response.json \\
    [--timeout 60000]

Requires:
- .browser-profiles/grok-state.json (auth state)

This uses Playwright directly (not MCP) to avoid context burn.
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

async function askGrok(question, storageStatePath, timeout = 60000) {
  // Launch browser - positioned offscreen to avoid disruption
  const browser = await chromium.launch({
    headless: false,  // Grok may detect headless browsers
    args: [
      '--disable-blink-features=AutomationControlled',
      '--no-sandbox',
      '--disable-setuid-sandbox',
      '--disable-dev-shm-usage',
      '--disable-web-security',
      '--window-position=3000,3000',  // Way offscreen
      '--window-size=1,1'              // Tiny window
    ]
  });

  const context = await browser.newContext({
    storageState: storageStatePath,
    viewport: { width: 1280, height: 800 },
    userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36'
  });

  const page = await context.newPage();

  console.error("✓ Browser launched (positioned offscreen)");

  try {
    const grokUrl = "https://x.com/i/grok";
    console.error(`Navigating to: ${grokUrl}`);
    await page.goto(grokUrl, { waitUntil: "domcontentloaded", timeout: 30000 });

    console.error("Waiting for Grok interface...");
    await page.waitForTimeout(3000); // Let UI load

    // Find the input textbox
    const input = page.locator('textbox[placeholder="Ask anything"], textarea[placeholder="Ask anything"], input[placeholder="Ask anything"]').first();
    await input.waitFor({ state: "visible", timeout: 10000 });

    console.error(`Typing question (${question.length} chars)...`);
    await input.fill(question);
    await page.waitForTimeout(500);

    // Submit by pressing Enter
    console.error("Submitting question...");
    await input.press('Enter');

    console.error("Waiting for response...");
    await page.waitForTimeout(3000); // Initial wait for response to start

    // Wait for response to complete
    // Strategy: Look for the response container and wait for text stability
    const startTime = Date.now();
    let lastLength = 0;
    let stableChecks = 0;
    const maxStabilityChecks = 3;

    while (stableChecks < maxStabilityChecks) {
      if (Date.now() - startTime > timeout) {
        throw new Error(`Timeout waiting for response after ${timeout}ms`);
      }

      await page.waitForTimeout(3000);

      // Try to get all text from the main conversation area
      // Grok responses typically appear in a conversation thread
      const mainContent = page.locator('main').first();
      const currentText = await mainContent.innerText().catch(() => "");
      const currentLength = currentText.length;

      if (currentLength === lastLength && currentLength > question.length) {
        stableChecks++;
        console.error(`Response stable (check ${stableChecks}/${maxStabilityChecks}), length: ${currentLength} chars`);
        if (stableChecks >= 2) {
          // Stable for 2 checks (6 seconds) is enough
          break;
        }
      } else if (currentLength > lastLength) {
        console.error(`Response growing: ${lastLength} -> ${currentLength} chars`);
        lastLength = currentLength;
        stableChecks = 0;
      }
    }

    console.error("Extracting response...");

    // Extract the response text
    // We need to get just the Grok response, not the entire page
    // Look for the conversation structure and extract the last assistant message
    const mainContent = page.locator('main').first();
    const fullText = await mainContent.innerText().catch(() => "");

    if (!fullText || fullText.length === 0) {
      throw new Error("Could not extract any text from Grok response");
    }

    console.error(`Extracted ${fullText.length} chars of text`);

    // Try to isolate just the response (remove the question)
    // This is a simple heuristic - we look for text after our question
    let responseText = fullText;
    const questionIndex = fullText.indexOf(question);
    if (questionIndex !== -1) {
      // Get everything after the question
      responseText = fullText.substring(questionIndex + question.length).trim();
      console.error(`Isolated response: ${responseText.length} chars`);
    }

    // Return structured response
    return {
      ok: true,
      question: question,
      answer: responseText,
      full_text: fullText,
      timestamp: new Date().toISOString()
    };

  } finally {
    // Close browser cleanly
    await page.close().catch(() => {});
    await context.close().catch(() => {});
    await browser.close().catch(() => {});
    console.error("✓ Browser closed");
  }
}

(async function main() {
  const args = parseArgs(process.argv);

  if (args.help || args.h) usage(0);

  const question = args.question;
  const outPath = args.out;
  const timeout = parseInt(args.timeout || "60000", 10);

  if (!question) {
    console.error("Missing required --question");
    usage(1);
  }

  // Check for storage state
  const storageStatePath = ".browser-profiles/grok-state.json";
  if (!fs.existsSync(storageStatePath)) {
    console.error(`Storage state not found: ${storageStatePath}`);
    console.error("You need to create Grok storage state first.");
    process.exit(1);
  }

  console.error("=== Grok Question ===");
  console.error(`Question: ${question}`);
  console.error(`Timeout: ${timeout}ms`);
  console.error("");

  const response = await askGrok(question, storageStatePath, timeout);

  const output = JSON.stringify(response, null, 2);

  if (outPath) {
    const abs = path.isAbsolute(outPath) ? outPath : path.join(process.cwd(), outPath);
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, output + "\n", "utf8");
    console.error(`✓ Response written to: ${abs}`);
  } else {
    process.stdout.write(output + "\n");
  }

  console.error("");
  console.error("✓ Complete");
  process.exit(0);
})().catch((err) => {
  console.error(`ERROR: ${err.message}`);
  process.exit(1);
});
