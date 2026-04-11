#!/usr/bin/env node
// TypeScript client codegen from OpenAPI specs.
// Usage: node tools/ts-codegen/ts-codegen.mjs <module> [module2 ...]
// Usage: node tools/ts-codegen/ts-codegen.mjs --all
// Usage: node tools/ts-codegen/ts-codegen.mjs --all --regen   (CI: regenerate all, incl. existing)
//
// Generates per clients/{module}/:
//   package.json, tsconfig.json  — only if not present (never overwrites)
//   src/{module}.d.ts            — always regenerated from openapi.json
//   src/index.ts                 — only if not present, or if --regen

import { execSync } from "node:child_process";
import {
  readFileSync,
  writeFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
} from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = join(__dirname, "../..");
const CLIENTS = join(ROOT, "clients");

// ── Helpers ─────────────────────────────────────────────────────────

/** fixed-assets → FixedAssets */
function toPascalCase(kebab) {
  return kebab
    .split("-")
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join("");
}

// ── Templates ───────────────────────────────────────────────────────

function makePackageJson(mod) {
  return {
    name: `@7d/${mod}-client`,
    version: "1.0.0",
    type: "module",
    description: `Generated TypeScript client for the ${toPascalCase(mod)} service`,
    main: "src/index.ts",
    scripts: {
      "generate:file": `openapi-typescript openapi.json -o src/${mod}.d.ts`,
      typecheck: "tsc --noEmit",
    },
    dependencies: {
      "openapi-fetch": "^0.13.0",
    },
    devDependencies: {
      "@types/node": "^25.5.0",
      "openapi-typescript": "^7.6.1",
      typescript: "^5.7.0",
    },
  };
}

const TSCONFIG = {
  compilerOptions: {
    target: "ES2022",
    module: "Node16",
    moduleResolution: "Node16",
    strict: true,
    esModuleInterop: true,
    skipLibCheck: true,
    outDir: "dist",
    rootDir: "src",
    declaration: true,
  },
  include: ["src"],
  exclude: ["dist", "node_modules"],
};

function makeIndexTs(mod, dtsFile, schemas) {
  const pascal = toPascalCase(mod);
  const lines = [
    "// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.",
    `import createClient from "openapi-fetch";`,
    `import type { paths, components } from "./${dtsFile}";`,
    "",
    `export type { paths, components } from "./${dtsFile}";`,
    "",
  ];

  if (schemas.length > 0) {
    lines.push("// ── Schema type re-exports ──────────────────────────────────────");
    for (const name of schemas) {
      // Skip names with generics — they can't be re-exported cleanly as aliases
      if (name.includes("<") || name.includes(">")) continue;
      lines.push(`export type ${name} = components["schemas"]["${name}"];`);
    }
    lines.push("");
  }

  lines.push(`export interface ${pascal}ClientOptions {`);
  lines.push("  baseUrl: string;");
  lines.push("  token: string;");
  lines.push("}");
  lines.push("");
  lines.push(
    `export function create${pascal}Client(opts: ${pascal}ClientOptions) {`
  );
  lines.push("  return createClient<paths>({");
  lines.push("    baseUrl: opts.baseUrl,");
  lines.push("    headers: {");
  lines.push("      Authorization: `Bearer ${opts.token}`,");
  lines.push('      "Content-Type": "application/json",');
  lines.push("    },");
  lines.push("  });");
  lines.push("}");
  lines.push("");

  return lines.join("\n");
}

// ── Generate one module ─────────────────────────────────────────────

function generate(mod, { regen = false } = {}) {
  const dir = join(CLIENTS, mod);
  const specPath = join(dir, "openapi.json");

  if (!existsSync(specPath)) {
    console.error(`✗ ${mod}: no openapi.json — skipping`);
    return false;
  }

  // Validate the spec is parseable JSON
  let spec;
  try {
    spec = JSON.parse(readFileSync(specPath, "utf8"));
  } catch (e) {
    console.error(`✗ ${mod}: invalid openapi.json (${e.message.slice(0, 60)}) — skipping`);
    return false;
  }

  const schemas = Object.keys(spec.components?.schemas || {});
  const dtsFile = `${mod}.d.ts`;
  const srcDir = join(dir, "src");

  // Ensure src/ exists
  mkdirSync(srcDir, { recursive: true });

  // 1. package.json — only if not present (preserves richer hand-crafted configs)
  if (!existsSync(join(dir, "package.json"))) {
    writeFileSync(
      join(dir, "package.json"),
      JSON.stringify(makePackageJson(mod), null, 2) + "\n"
    );
  }

  // 2. tsconfig.json — only if not present
  if (!existsSync(join(dir, "tsconfig.json"))) {
    writeFileSync(
      join(dir, "tsconfig.json"),
      JSON.stringify(TSCONFIG, null, 2) + "\n"
    );
  }

  // 3. npm install — only if node_modules is missing
  if (!existsSync(join(dir, "node_modules"))) {
    console.log(`  ${mod}: installing dependencies …`);
    execSync("npm install --cache /tmp/npm-cache-codegen", {
      cwd: dir,
      stdio: ["ignore", "ignore", "inherit"],
    });
  }

  // 4. openapi-typescript → .d.ts (always regenerate from spec)
  const oatsBin = join(dir, "node_modules", ".bin", "openapi-typescript");
  console.log(`  ${mod}: generating ${dtsFile} …`);
  execSync(
    `"${oatsBin}" "${specPath}" -o "${join(srcDir, dtsFile)}"`,
    { cwd: dir, stdio: ["ignore", "ignore", "inherit"] }
  );

  // 5. index.ts — only if not present or --regen
  const indexPath = join(srcDir, "index.ts");
  if (!existsSync(indexPath) || regen) {
    console.log(`  ${mod}: generating index.ts …`);
    writeFileSync(indexPath, makeIndexTs(mod, dtsFile, schemas));
  } else {
    console.log(`  ${mod}: index.ts exists, skipping (use --regen to overwrite)`);
  }

  // 6. tsc --noEmit
  console.log(`  ${mod}: type-checking …`);
  try {
    const tscBin = join(dir, "node_modules", ".bin", "tsc");
    execSync(`"${tscBin}" --noEmit`, { cwd: dir, stdio: "pipe" });
  } catch (e) {
    const out = e.stdout?.toString().trim() || "";
    const err = e.stderr?.toString().trim() || "";
    console.error(`✗ ${mod}: tsc failed:`);
    if (out) console.error(out);
    if (err) console.error(err);
    return false;
  }

  console.log(`✓ ${mod}`);
  return true;
}

// ── Main ────────────────────────────────────────────────────────────

const args = process.argv.slice(2);
const regen = args.includes("--regen");
const allModules = args.includes("--all");
let modules = args.filter((a) => !a.startsWith("--"));

if (args.length === 0 || (allModules && modules.length === 0 && !regen && args.every(a => a === "--all" || a === "--regen"))) {
  if (!allModules) {
    console.error(
      "Usage: node tools/ts-codegen/ts-codegen.mjs <module> [module2 ...]\n" +
        "       node tools/ts-codegen/ts-codegen.mjs --all\n" +
        "       node tools/ts-codegen/ts-codegen.mjs --all --regen"
    );
    process.exit(1);
  }
}

if (allModules) {
  modules = readdirSync(CLIENTS)
    .filter((m) => {
      if (!existsSync(join(CLIENTS, m, "openapi.json"))) return false;
      // Without --regen, skip modules that already have an index.ts (hand-built or previously generated)
      if (!regen && existsSync(join(CLIENTS, m, "src", "index.ts"))) return false;
      return true;
    })
    .sort();
}

if (modules.length === 0) {
  console.log("No modules to generate (all up to date). Use --regen to force.");
  process.exit(0);
}

console.log(`Generating TS clients for: ${modules.join(", ")}\n`);

let ok = 0;
let fail = 0;
for (const mod of modules) {
  if (generate(mod, { regen })) ok++;
  else fail++;
}

console.log(`\nDone: ${ok} succeeded, ${fail} failed`);
if (fail > 0) process.exit(1);
