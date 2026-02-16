#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import Ajv2020 from "ajv/dist/2020.js";
import addFormats from "ajv-formats";

function usage(exitCode = 1) {
  console.error(`
packet-validate.mjs

Validate a Flywheel ChatGPT protocol packet (request or response) against local schemas.

Usage:
  node scripts/chatgpt/packet-validate.mjs --file path/to/packet.json
  cat packet.json | node scripts/chatgpt/packet-validate.mjs --stdin

Exit codes:
  0 = valid
  2 = invalid (schema errors)
  1 = tool error

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

function readAllStdin() {
  return new Promise((resolve, reject) => {
    let data = "";
    process.stdin.setEncoding("utf8");
    process.stdin.on("data", (chunk) => (data += chunk));
    process.stdin.on("end", () => resolve(data));
    process.stdin.on("error", reject);
  });
}

function readJson(raw, label = "input") {
  try {
    return JSON.parse(raw);
  } catch (e) {
    throw new Error(`Invalid JSON in ${label}: ${e.message}`);
  }
}

function schemaDirFromThisScript() {
  const __filename = new URL(import.meta.url).pathname;
  const __dirname = path.dirname(__filename);
  // scripts/chatgpt -> project root -> schemas/...
  return path.resolve(__dirname, "..", "..", "schemas", "flywheel", "chatgpt", "v1");
}

function loadSchemas(ajv, dir) {
  const files = fs.readdirSync(dir).filter((f) => f.endsWith(".schema.json"));
  for (const f of files) {
    const p = path.join(dir, f);
    const raw = fs.readFileSync(p, "utf8");
    const schema = readJson(raw, p);
    // Ensure schemas are resolvable by $id and by filename ref.
    ajv.addSchema(schema, schema.$id || f);
  }
}

function schemaIdForMsgType(msgType) {
  const map = {
    // Requests
    RFP_PLAN: "flywheel.chatgpt.v1/msg-rfp-plan.schema.json",
    RFP_ARBITRATE: "flywheel.chatgpt.v1/msg-rfp-arbitrate.schema.json",
    EVIDENCE_BUNDLE: "flywheel.chatgpt.v1/msg-evidence-bundle.schema.json",
    SPEC_LOCK: "flywheel.chatgpt.v1/msg-spec-lock.schema.json",
    ACCEPTANCE_GATE: "flywheel.chatgpt.v1/msg-acceptance-gate.schema.json",
    // Responses
    RFP_PLAN_RESPONSE: "flywheel.chatgpt.v1/resp-rfp-plan.schema.json",
    RFP_ARBITRATE_RESPONSE: "flywheel.chatgpt.v1/resp-rfp-arbitrate.schema.json",
    EVIDENCE_BUNDLE_RESPONSE: "flywheel.chatgpt.v1/resp-evidence-bundle.schema.json",
    SPEC_LOCK_RESPONSE: "flywheel.chatgpt.v1/resp-spec-lock.schema.json",
    ACCEPTANCE_GATE_RESPONSE: "flywheel.chatgpt.v1/resp-acceptance-gate.schema.json"
  };
  return map[msgType] || null;
}

function formatErrors(errors) {
  return (errors || [])
    .map((e) => {
      const inst = e.instancePath || "(root)";
      const msg = e.message || "schema error";
      const extra = e.params ? ` params=${JSON.stringify(e.params)}` : "";
      return `- ${inst}: ${msg}${extra}`;
    })
    .join("\n");
}

(async function main() {
  const args = parseArgs(process.argv);
  if (args.help || args.h) usage(0);

  const fromStdin = !!args.stdin;
  const filePath = args.file;

  if (!fromStdin && !filePath) {
    console.error("Provide --file or --stdin.");
    usage(1);
  }
  if (fromStdin && filePath) {
    console.error("Use only one of --file or --stdin.");
    usage(1);
  }

  let raw;
  let label;
  if (fromStdin) {
    raw = await readAllStdin();
    label = "stdin";
  } else {
    const abs = path.isAbsolute(filePath) ? filePath : path.join(process.cwd(), filePath);
    raw = fs.readFileSync(abs, "utf8");
    label = abs;
  }

  const packet = readJson(raw, label);
  if (!packet || typeof packet !== "object") throw new Error("Packet must be a JSON object.");

  const msgType = packet.msg_type;
  if (!msgType || typeof msgType !== "string") {
    console.error("Invalid packet: missing string field msg_type.");
    process.exit(2);
  }

  const schemaId = schemaIdForMsgType(msgType);
  if (!schemaId) {
    console.error(`Unknown msg_type "${msgType}". No schema mapping exists.`);
    process.exit(2);
  }

  const ajv = new Ajv2020({
    strict: false,
    allErrors: true,
    allowUnionTypes: true
  });
  addFormats(ajv);

  const dir = schemaDirFromThisScript();
  loadSchemas(ajv, dir);

  const validate = ajv.getSchema(schemaId);
  if (!validate) {
    console.error(`Schema not found/loaded: ${schemaId}`);
    process.exit(1);
  }

  const ok = validate(packet);
  if (ok) {
    process.stdout.write(`OK: ${msgType} (${label})\n`);
    process.exit(0);
  } else {
    process.stderr.write(`INVALID: ${msgType} (${label})\n`);
    process.stderr.write(formatErrors(validate.errors) + "\n");
    process.exit(2);
  }
})().catch((err) => {
  console.error(`packet-validate error: ${err.message}`);
  process.exit(1);
});
