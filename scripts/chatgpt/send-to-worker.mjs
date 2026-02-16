#!/usr/bin/env node
import fs from "node:fs";

/**
 * Send a message to the browser worker
 *
 * Usage:
 *   node scripts/chatgpt/send-to-worker.mjs \
 *     --message-file tmp/message.txt \
 *     --conversation-url https://chatgpt.com/c/... \
 *     --out tmp/response.json
 */

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

const args = parseArgs(process.argv);

const messageFile = args["message-file"];
const conversationUrl = args["conversation-url"];
const outFile = args["out"];
const timeout = parseInt(args.timeout || "120000", 10);

if (!messageFile || !conversationUrl) {
  console.error("Usage: send-to-worker.mjs --message-file FILE --conversation-url URL [--out FILE]");
  process.exit(1);
}

const REQUEST_FILE = ".flywheel/browser-request.json";
const RESPONSE_FILE = ".flywheel/browser-response.json";
const READY_FILE = ".flywheel/browser-ready.txt";

// Check if worker is ready
if (!fs.existsSync(READY_FILE)) {
  console.error("ERROR: Browser worker not running");
  console.error("Start with: node scripts/chatgpt/browser-worker.mjs");
  process.exit(1);
}

// Read message
const message = fs.readFileSync(messageFile, "utf8");

// Delete old response if exists
if (fs.existsSync(RESPONSE_FILE)) {
  fs.unlinkSync(RESPONSE_FILE);
}

// Write request
const request = {
  message: message,
  conversation_url: conversationUrl
};

fs.writeFileSync(REQUEST_FILE, JSON.stringify(request, null, 2) + "\n");
console.error(`Request sent (${message.length} chars)`);

// Wait for response
const startTime = Date.now();
while (true) {
  if (fs.existsSync(RESPONSE_FILE)) {
    const response = fs.readFileSync(RESPONSE_FILE, "utf8");

    if (outFile) {
      fs.writeFileSync(outFile, response);
      console.error(`✓ Response written to: ${outFile}`);
    } else {
      console.log(response);
    }

    process.exit(0);
  }

  if (Date.now() - startTime > timeout) {
    console.error("ERROR: Timeout waiting for response");
    process.exit(1);
  }

  await new Promise(resolve => setTimeout(resolve, 200));
}
