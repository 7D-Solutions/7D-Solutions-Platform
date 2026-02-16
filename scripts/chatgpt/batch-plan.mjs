#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { execSync } from "node:child_process";

function usage(exitCode = 1) {
  console.error(`
batch-plan.mjs

Send multiple bead contexts to ChatGPT at once and get back multiple plans.

Usage:
  node scripts/chatgpt/batch-plan.mjs \\
    --beads bd-auth,bd-api,bd-ui \\
    --conversation-url https://chatgpt.com/c/... \\
    [--out plans/batch-001.json]

The tool will:
1. Read each bead's context using 'br show <bead-id>'
2. Format a batch request to ChatGPT
3. Extract multiple plan responses
4. Output as JSON array

Output format:
[
  {
    "bead_id": "bd-auth",
    "plan": [...],
    "risks": [...],
    "acceptance_tests": [...],
    "next_actions": [...]
  },
  ...
]
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

function getBeadInfo(beadId) {
  try {
    const output = execSync(`br show ${beadId} --format json`, { encoding: "utf8" });
    return JSON.parse(output);
  } catch (e) {
    console.error(`Failed to get info for bead ${beadId}: ${e.message}`);
    return null;
  }
}

function formatBatchRequest(beads) {
  const beadContexts = beads.map((b, idx) => {
    return `
## Bead ${idx + 1}: ${b.id}
**Title:** ${b.title || b.subject || "No title"}
**Status:** ${b.status || "unknown"}
**Description:**
${b.description || b.body || "No description"}

**Dependencies:** ${b.dependencies?.length ? b.dependencies.join(", ") : "none"}
`;
  }).join("\n---\n");

  return `I need implementation plans for ${beads.length} beads. For EACH bead, provide a plan following this exact format:

{
  "id": "bd-XXXX",
  "code": "pXa-01",
  "title": "Short action-oriented title",
  "priority": "P0|P1|P2|P3",
  "depends_on": ["bd-YYYY"],

  "how_to_think": "One focused paragraph explaining the intent of this bead, what must be preserved (idempotency, transaction boundaries, additive changes), and when to stop/escalate. CRITICAL: This helps pool-based agents understand the mindset needed - are they debugging? refactoring? exploring? building new features?",

  "acceptance_criteria": [
    "Clear measurable outcome",
    "Another measurable outcome"
  ],

  "files_to_create": ["path/to/new-file.ts"],
  "files_to_modify": ["path/to/existing-file.ts"],

  "verification": [
    "Docker-first or concrete command to run",
    "What must be true afterward (expected output, state, test results)"
  ]
}

Return as valid JSON array where each element corresponds to a bead in order:

[
  {
    "id": "${beads[0]?.id}",
    "code": "...",
    "title": "...",
    ...
  },
  // ... one object per bead
]

${beadContexts}

Return your response in this EXACT format with no additional text:

\`\`\`json
[
  { "id": "bd-...", ... }
]
\`\`\`
`;
}

(async function main() {
  const args = parseArgs(process.argv);

  if (args.help || args.h) usage(0);

  const beadsArg = args.beads;
  const conversationUrl = args["conversation-url"];
  const outPath = args.out;

  if (!beadsArg) {
    console.error("Missing required --beads argument");
    usage(1);
  }

  if (!conversationUrl) {
    console.error("Missing required --conversation-url argument");
    usage(1);
  }

  const beadIds = beadsArg.split(",").map(s => s.trim());

  // Guard #5: Cap batch size at 5 beads for MVP
  const MAX_BATCH_SIZE = 5;
  if (beadIds.length > MAX_BATCH_SIZE) {
    console.error(`ERROR: Batch size ${beadIds.length} exceeds maximum of ${MAX_BATCH_SIZE}`);
    console.error(`Please split into smaller batches to avoid truncation/continue behavior.`);
    process.exit(1);
  }

  console.error(`Fetching info for ${beadIds.length} beads...`);
  const beads = beadIds.map(id => {
    const info = getBeadInfo(id);
    return info ? { id, ...info } : { id, title: "Failed to load", description: "Error loading bead info" };
  });

  console.error(`Formatting batch request for ChatGPT...`);
  const request = formatBatchRequest(beads);

  // Save request to temp file
  fs.mkdirSync("tmp", { recursive: true });
  const requestFile = "tmp/batch-request.txt";
  fs.writeFileSync(requestFile, request, "utf8");
  console.error(`Request saved to ${requestFile} (${request.length} chars)`);

  // Call send-to-worker.mjs to use the persistent browser worker
  console.error("");
  console.error("Sending to browser worker...");

  const responseFile = outPath || "tmp/batch-response.json";
  try {
    execSync(
      `node scripts/chatgpt/send-to-worker.mjs --message-file "${requestFile}" --conversation-url "${conversationUrl}" --out "${responseFile}" --timeout 120000`,
      { encoding: "utf8", stdio: "inherit" }
    );
  } catch (e) {
    console.error(`Failed to send to worker: ${e.message}`);
    console.error(`Hint: Check if browser worker is running with: ./scripts/chatgpt/check-worker.sh`);
    process.exit(1);
  }

  // Read the response
  if (!fs.existsSync(responseFile)) {
    console.error(`Response file not created: ${responseFile}`);
    process.exit(1);
  }

  const extractionResult = JSON.parse(fs.readFileSync(responseFile, "utf8"));

  console.error("");

  let output;

  if (extractionResult.parse_ok) {
    // Happy path: JSON parsed successfully
    output = JSON.stringify(extractionResult.extracted_json, null, 2);
    console.error(`✓ JSON parsed successfully (${output.length} chars)`);
    console.error(`✓ Raw text size: ${extractionResult.raw_text.length} chars`);
    console.error(`✓ Context burn check: Returning structured JSON`);
  } else {
    // Error path: Return raw text + error for agent to handle
    const errorResponse = {
      error: extractionResult.error || "PARSE_FAILED",
      raw_text: extractionResult.raw_text,
      message: "ChatGPT response could not be parsed as JSON. Agent should review raw_text and decide how to proceed."
    };
    output = JSON.stringify(errorResponse, null, 2);
    console.error(`✗ JSON parsing failed: ${extractionResult.error}`);
    console.error(`✗ Raw text size: ${extractionResult.raw_text.length} chars`);
    console.error(`✗ Returning error response with raw text for agent review`);
  }

  if (outPath) {
    const abs = path.isAbsolute(outPath) ? outPath : path.join(process.cwd(), outPath);
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, output + "\n", "utf8");
    console.error(`✓ Response written to: ${abs}`);
  } else {
    process.stdout.write(output + "\n");
  }

  console.error("");
  console.error("=== Batch Planning Complete ===");

  process.exit(0);
})().catch((err) => {
  console.error(`batch-plan error: ${err.message}`);
  process.exit(1);
});
