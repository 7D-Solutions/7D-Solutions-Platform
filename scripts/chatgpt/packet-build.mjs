#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";

function usage(exitCode = 1) {
  console.error(`
packet-build.mjs

Build a Flywheel ChatGPT protocol request packet (v1).

Usage:
  node scripts/chatgpt/packet-build.mjs \\
    --type RFP_PLAN \\
    --bead bd-xxxx \\
    --sender QuietDune \\
    --context path/to/context.json \\
    --question "What should we do next?" \\
    [--question "Second question..."] \\
    [--artifact path/to/artifact.json] \\
    [--idempotency-key "bd-xxxx:RFP_PLAN:..."] \\
    [--out path/to/packet.json]

Notes:
- --context must be a JSON file matching: { repo, branch, goal, constraints[] }
- --artifact files must be JSON matching: { type, label, content } (content is a string)
`);
  process.exit(exitCode);
}

function parseArgs(argv) {
  const args = {};
  const rest = [];
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith("--")) {
      const key = a.slice(2);
      const next = argv[i + 1];
      if (!next || next.startsWith("--")) {
        args[key] = true;
      } else {
        if (args[key] === undefined) args[key] = next;
        else if (Array.isArray(args[key])) args[key].push(next);
        else args[key] = [args[key], next];
        i++;
      }
    } else {
      rest.push(a);
    }
  }
  args._ = rest;
  return args;
}

function readJsonFile(p) {
  const raw = fs.readFileSync(p, "utf8");
  try {
    return JSON.parse(raw);
  } catch (e) {
    throw new Error(`Invalid JSON in ${p}: ${e.message}`);
  }
}

function nowIso() {
  return new Date().toISOString();
}

const DEFAULT_FIELDS_BY_TYPE = {
  RFP_PLAN: ["plan", "risks", "acceptance_tests", "next_actions"],
  RFP_ARBITRATE: ["decision", "rationale", "tradeoffs", "next_actions"],
  EVIDENCE_BUNDLE: ["findings", "missing", "next_actions"],
  SPEC_LOCK: ["spec", "next_actions"],
  ACCEPTANCE_GATE: ["acceptance", "next_actions"]
};

const VALID_TYPES = Object.keys(DEFAULT_FIELDS_BY_TYPE);

(async function main() {
  const args = parseArgs(process.argv);

  if (args.help || args.h) usage(0);

  const type = args.type;
  const bead = args.bead;
  const sender = args.sender;
  const contextPath = args.context;
  const outPath = args.out;

  if (!type || !bead || !sender || !contextPath) {
    console.error("Missing required args.");
    usage(1);
  }
  if (!VALID_TYPES.includes(type)) {
    console.error(`Invalid --type "${type}". Valid: ${VALID_TYPES.join(", ")}`);
    process.exit(2);
  }

  const context = readJsonFile(contextPath);
  if (!context || typeof context !== "object") throw new Error("Context must be an object JSON.");
  const requiredContextKeys = ["repo", "branch", "goal", "constraints"];
  for (const k of requiredContextKeys) {
    if (!(k in context)) throw new Error(`Context missing required key: ${k}`);
  }
  if (!Array.isArray(context.constraints)) throw new Error("Context.constraints must be an array.");

  const questions = [];
  if (args.question) {
    if (Array.isArray(args.question)) questions.push(...args.question);
    else questions.push(args.question);
  }

  const artifacts = [];
  if (args.artifact) {
    const artifactPaths = Array.isArray(args.artifact) ? args.artifact : [args.artifact];
    for (const ap of artifactPaths) {
      const art = readJsonFile(ap);
      artifacts.push(art);
    }
  }

  const packet = {
    proto: "flywheel.chatgpt.v1",
    bead_id: bead,
    msg_type: type,
    sender,
    ts: nowIso(),
    context,
    inputs: {
      artifacts,
      questions
    },
    requested_output: {
      format: "json",
      fields: DEFAULT_FIELDS_BY_TYPE[type]
    }
  };

  if (args["idempotency-key"] && typeof args["idempotency-key"] === "string") {
    packet.idempotency_key = args["idempotency-key"];
  }

  const json = JSON.stringify(packet, null, 2);

  if (outPath) {
    const abs = path.isAbsolute(outPath) ? outPath : path.join(process.cwd(), outPath);
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, json + "\n", "utf8");
  } else {
    process.stdout.write(json + "\n");
  }
})().catch((err) => {
  console.error(`packet-build error: ${err.message}`);
  process.exit(1);
});
