#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { execSync } from "node:child_process";
import { chromium } from "playwright";

/**
 * Post a message to ChatGPT and extract JSON response
 *
 * Uses Playwright DIRECTLY (not MCP) to:
 * - Load authenticated ChatGPT session
 * - Post message
 * - Wait for response
 * - Extract JSON code block
 * - Return it
 *
 * This runs as a separate process so it doesn't burn the bridge agent's context.
 */

function usage(exitCode = 1) {
  console.error(`
post-and-extract.mjs

Post a message to ChatGPT and extract the JSON response.

Usage:
  node scripts/chatgpt/post-and-extract.mjs \\
    --message-file tmp/batch-request.txt \\
    --conversation-url https://chatgpt.com/c/... \\
    --out tmp/batch-response.json \\
    [--timeout 60000]

Requires:
- .browser-profiles/chatgpt-state.json (auth state)

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

async function postAndExtract(conversationUrl, message, storageStatePath, timeout = 60000) {
  // Launch browser - use window positioning to keep it offscreen immediately
  const browser = await chromium.launch({
    headless: false,  // MUST be false - ChatGPT blocks headless
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

  // Hide window using osascript in parallel (belt and suspenders)
  const hidePromise = (async () => {
    await new Promise(resolve => setTimeout(resolve, 200));
    try {
      execSync('osascript -e \'tell application "System Events" to set visible of process "Chromium" to false\'', { timeout: 1000 });
    } catch (e) {
      try {
        execSync('osascript -e \'tell application "System Events" to set visible of process "Google Chrome" to false\'', { timeout: 1000 });
      } catch (e2) {
        // Ignore - window position should keep it hidden anyway
      }
    }
  })();

  const context = await browser.newContext({
    storageState: storageStatePath,
    viewport: { width: 1280, height: 800 },
    userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36'
  });

  const page = await context.newPage();

  await hidePromise; // Wait for hide attempt to complete
  console.error("✓ Browser launched (positioned offscreen)");

  try {
    console.error(`Navigating to: ${conversationUrl}`);
    await page.goto(conversationUrl, { waitUntil: "domcontentloaded", timeout: 30000 });

    console.error("Waiting for input...");
    const input = page.locator('[contenteditable="true"]').last();
    await input.waitFor({ state: "visible", timeout: 10000 });

    // Wait for messages to load
    await page.waitForTimeout(2000);

    // Guard #1: Capture baseline count BEFORE sending to avoid race condition
    console.error("Capturing baseline message count...");
    await page.waitForTimeout(500);  // Wait for page to stabilize
    const baselineCount = await page.locator('[data-message-author-role="assistant"]').count();
    console.error(`Baseline assistant messages: ${baselineCount}`);

    console.error(`Posting message (${message.length} chars)...`);
    await input.evaluate((el, text) => {
      el.textContent = text;
      el.dispatchEvent(new Event("input", { bubbles: true }));
    }, message);

    await page.waitForTimeout(1000);

    // Press Enter to send (more reliable than clicking button)
    await input.press('Enter');

    console.error("Waiting for new assistant message...");

    // Wait for new message to appear (count increments)
    const messageLocator = page.locator('[data-message-author-role="assistant"]');
    let newMessageAppeared = false;
    const waitStart = Date.now();

    while (!newMessageAppeared && (Date.now() - waitStart < 10000)) {
      const currentCount = await messageLocator.count();
      if (currentCount > baselineCount) {
        newMessageAppeared = true;
        console.error(`New message appeared (count: ${currentCount})`);
      } else {
        await page.waitForTimeout(500);
      }
    }

    if (!newMessageAppeared) {
      throw new Error("Timeout waiting for new assistant message to appear");
    }

    console.error("Waiting for response to complete...");

    // Hybrid completion detection:
    // 1. Wait for stop button to disappear
    // 2. Wait 5 seconds for initial DOM settle
    // 3. Verify text stability (no changes for 3 seconds)
    const stopButton = page.locator('button').filter({ hasText: /stop generating/i });

    const startTime = Date.now();
    let stopButtonGone = false;

    // Step 1: Wait for stop button to disappear
    while (!stopButtonGone && (Date.now() - startTime < timeout)) {
      await page.waitForTimeout(3000);

      const isGenerating = await stopButton.isVisible().catch(() => false);

      if (!isGenerating) {
        console.error("Stop button gone, initial DOM settle...");
        stopButtonGone = true;
      } else {
        console.error("Still generating...");
      }
    }

    if (!stopButtonGone) {
      throw new Error(`Timeout waiting for generation to complete after ${timeout}ms`);
    }

    // Step 2: Wait 5 seconds for initial DOM settle
    await page.waitForTimeout(5000);

    // Step 3: Verify text stability (check every 3 seconds, max 3 checks)
    const lastMessage = page.locator('[data-message-author-role="assistant"]').last();
    let lastText = await lastMessage.innerText().catch(() => "");
    let stableChecks = 0;
    let emptyChecks = 0; // Guard #9: Count consecutive empty checks
    const maxStabilityChecks = 3;

    console.error(`Initial text length: ${lastText.length} chars`);

    while (stableChecks < maxStabilityChecks) {
      await page.waitForTimeout(3000);

      const currentText = await lastMessage.innerText().catch(() => "");

      if (currentText === lastText && currentText.length > 0) {
        stableChecks++;
        emptyChecks = 0; // Reset empty counter when we have content
        console.error(`Text stable (check ${stableChecks}/${maxStabilityChecks})`);
        if (stableChecks >= 2) {
          // Stable for 2 checks (6 seconds) is enough
          break;
        }
      } else if (currentText.length === 0 && lastText.length === 0) {
        // Guard #9: Empty message detection
        emptyChecks++;
        console.error(`Text empty (check ${emptyChecks}/2)`);
        if (emptyChecks >= 2) {
          // Empty for 2 checks (6 seconds) - likely a failed/duplicate request
          console.error("Message remains empty after stability wait - treating as empty response");
          break;
        }
      } else {
        console.error(`Text changed: ${lastText.length} -> ${currentText.length} chars`);
        lastText = currentText;
        stableChecks = 0;
        emptyChecks = 0;
      }
    }

    console.error(`Final text length: ${lastText.length} chars, extracting...`);

    console.error("Extracting response...");

    // Use the text we already verified is stable
    let rawText = lastText;

    // Double-check with textContent as fallback
    if (!rawText || rawText.length === 0) {
      console.error("Stable text empty, trying textContent...");
      rawText = await lastMessage.textContent().catch(() => "");
    }

    // Guard #9: If last message is empty, try second-to-last (handles duplicates/failed generations)
    if (!rawText || rawText.length === 0) {
      console.error("Last message empty - trying second-to-last message (possible duplicate request)");
      const allMessages = await page.locator('[data-message-author-role="assistant"]').all();
      if (allMessages.length >= 2) {
        const secondToLast = allMessages[allMessages.length - 2];
        rawText = await secondToLast.innerText().catch(() => "");
        if (rawText && rawText.length > 0) {
          console.error(`✓ Found text in second-to-last message (${rawText.length} chars)`);
        }
      }
    }

    if (!rawText || rawText.length === 0) {
      throw new Error("Could not extract any text from last or second-to-last assistant message");
    }

    console.error(`Extracted ${rawText.length} chars of raw text (verified stable)`);

    // Best-effort JSON extraction
    let extractedJson = null;
    let parseOk = false;
    let error = null;

    try {
      // Try to find JSON in code fences first
      const jsonFenceMatches = [...rawText.matchAll(/```json\s*([\s\S]*?)\s*```/gi)];
      const genericFenceMatches = [...rawText.matchAll(/```\s*([\s\S]*?)\s*```/g)];

      let jsonText = null;

      if (jsonFenceMatches.length > 0) {
        // Use last json fence
        jsonText = jsonFenceMatches[jsonFenceMatches.length - 1][1];
        console.error("Found JSON in code fence");
      } else if (genericFenceMatches.length > 0) {
        // Try last generic fence
        jsonText = genericFenceMatches[genericFenceMatches.length - 1][1];
        console.error("Found generic code fence, attempting parse");
      } else {
        // No fences - try to extract JSON from raw text
        console.error("No code fences, searching for JSON structure");

        // Remove "JSON" prefix if present
        let cleaned = rawText.replace(/^JSON/i, '').trim();

        // Find all potential JSON structures (arrays or objects)
        const jsonCandidates = [];

        // Look for array start
        let arrayStart = cleaned.indexOf('[');
        if (arrayStart !== -1) {
          let depth = 0;
          let inString = false;
          let escape = false;

          for (let i = arrayStart; i < cleaned.length; i++) {
            const char = cleaned[i];
            if (escape) {
              escape = false;
              continue;
            }
            if (char === '\\') {
              escape = true;
              continue;
            }
            if (char === '"') {
              inString = !inString;
              continue;
            }
            if (inString) continue;

            if (char === '[') depth++;
            if (char === ']') {
              depth--;
              if (depth === 0) {
                jsonCandidates.push(cleaned.substring(arrayStart, i + 1));
                break;
              }
            }
          }
        }

        // Take the largest candidate (most likely to be complete)
        if (jsonCandidates.length > 0) {
          jsonText = jsonCandidates.reduce((a, b) => a.length > b.length ? a : b);
          console.error(`Found ${jsonCandidates.length} JSON candidate(s), using largest (${jsonText.length} chars)`);
        }
      }

      if (jsonText) {
        extractedJson = JSON.parse(jsonText);
        parseOk = true;
        console.error("✓ JSON parsed successfully");
      } else {
        error = "NO_JSON_STRUCTURE_FOUND";
        console.error("✗ No JSON structure found in response");
      }
    } catch (e) {
      error = `JSON_PARSE_ERROR: ${e.message}`;
      console.error(`✗ JSON parse failed: ${e.message}`);
    }

    // Return structured response with both raw text and parsed JSON
    return {
      ok: true,
      raw_text: rawText,
      parse_ok: parseOk,
      extracted_json: extractedJson,
      error: error
    };

  } finally {
    // Close browser cleanly (it was offscreen anyway)
    await page.close().catch(() => {});
    await context.close().catch(() => {});
    await browser.close().catch(() => {});
    console.error("✓ Browser closed (was offscreen)");
  }
}

(async function main() {
  const args = parseArgs(process.argv);

  if (args.help || args.h) usage(0);

  const messageFile = args["message-file"];
  const conversationUrl = args["conversation-url"];
  const outPath = args.out;
  const timeout = parseInt(args.timeout || "60000", 10);

  if (!messageFile) {
    console.error("Missing required --message-file");
    usage(1);
  }

  if (!conversationUrl) {
    console.error("Missing required --conversation-url");
    usage(1);
  }

  // Read message
  const message = fs.readFileSync(messageFile, "utf8");

  // Check for storage state
  const storageStatePath = ".browser-profiles/chatgpt-state.json";
  if (!fs.existsSync(storageStatePath)) {
    console.error(`Storage state not found: ${storageStatePath}`);
    console.error("Run: node scripts/init-chatgpt-storage-state.mjs");
    process.exit(1);
  }

  console.error("=== ChatGPT Post and Extract ===");
  console.error(`Conversation: ${conversationUrl}`);
  console.error(`Message length: ${message.length} chars`);
  console.error(`Timeout: ${timeout}ms`);
  console.error("");

  const response = await postAndExtract(conversationUrl, message, storageStatePath, timeout);

  const output = JSON.stringify(response, null, 2);

  if (outPath) {
    const abs = path.isAbsolute(outPath) ? outPath : path.join(process.cwd(), outPath);
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, output + "\n", "utf8");
    console.error(`✓ Response written to: ${abs}`);
  } else {
    process.stdout.write(output + "\n");
  }

  // Clean exit (persistent browser still running)
  console.error("");
  console.error("✓ Complete (reused persistent browser)");
  process.exit(0);
})().catch((err) => {
  console.error(`ERROR: ${err.message}`);
  process.exit(1);
});
