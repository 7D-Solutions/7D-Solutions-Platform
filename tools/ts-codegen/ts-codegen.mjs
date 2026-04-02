#!/usr/bin/env node
// TypeScript client codegen from OpenAPI specs.
// Usage: node tools/ts-codegen/ts-codegen.mjs <module> [module2 ...]
// Usage: node tools/ts-codegen/ts-codegen.mjs --all
//
// Generates: src/{module}.d.ts, src/index.ts, package.json, tsconfig.json
// per client directory under clients/{module}/.

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

/** fixed-assets → fixed_assets (for .d.ts filename) */
function toSnakeCase(kebab) {
  return kebab.replace(/-/g, "_");
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
    "// ── Schema type re-exports ──────────────────────────────────────",
  ];

  for (const name of schemas) {
    const alias = name.replace(/<.*>/, "");
    lines.push(`export type ${alias} = components["schemas"]["${name}"];`);
  }

  lines.push("");
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

function generate(mod) {
  const dir = join(CLIENTS, mod);
  const specPath = join(dir, "openapi.json");

  if (!existsSync(specPath)) {
    console.error(`✗ ${mod}: no openapi.json — skipping`);
    return false;
  }

  const spec = JSON.parse(readFileSync(specPath, "utf8"));
  const schemas = Object.keys(spec.components?.schemas || {});
  if (schemas.length === 0) {
    console.error(`✗ ${mod}: no schemas in spec — skipping`);
    return false;
  }

  const dtsFile = `${mod}.d.ts`;
  const srcDir = join(dir, "src");

  // Ensure src/ exists
  mkdirSync(srcDir, { recursive: true });

  // 1. package.json (write first so npm install gets openapi-typescript)
  if (!existsSync(join(dir, "package.json"))) {
    writeFileSync(
      join(dir, "package.json"),
      JSON.stringify(makePackageJson(mod), null, 2) + "\n"
    );
  }

  // 2. tsconfig.json
  if (!existsSync(join(dir, "tsconfig.json"))) {
    writeFileSync(
      join(dir, "tsconfig.json"),
      JSON.stringify(TSCONFIG, null, 2) + "\n"
    );
  }

  // 3. npm install (gets openapi-typescript locally)
  console.log(`  ${mod}: installing dependencies …`);
  execSync("npm install --cache /tmp/npm-cache-codegen", {
    cwd: dir,
    stdio: ["ignore", "ignore", "inherit"],
  });

  // 4. openapi-typescript → .d.ts (use local binary)
  const oatsBin = join(dir, "node_modules", ".bin", "openapi-typescript");
  console.log(`  ${mod}: generating ${dtsFile} …`);
  execSync(
    `"${oatsBin}" "${specPath}" -o "${join(srcDir, dtsFile)}"`,
    { cwd: dir, stdio: ["ignore", "ignore", "inherit"] }
  );

  // 5. index.ts
  console.log(`  ${mod}: generating index.ts …`);
  writeFileSync(join(srcDir, "index.ts"), makeIndexTs(mod, dtsFile, schemas));

  // 6. tsc --noEmit
  console.log(`  ${mod}: type-checking …`);
  try {
    execSync("npx tsc --noEmit", { cwd: dir, stdio: "pipe" });
  } catch (e) {
    console.error(`✗ ${mod}: tsc failed:\n${e.stdout?.toString()}`);
    return false;
  }

  console.log(`✓ ${mod}`);
  return true;
}

// ── Main ────────────────────────────────────────────────────────────

let modules = process.argv.slice(2);

if (modules.length === 0) {
  console.error(
    "Usage: node tools/ts-codegen/ts-codegen.mjs <module> [module2 ...]\n" +
      "       node tools/ts-codegen/ts-codegen.mjs --all"
  );
  process.exit(1);
}

if (modules.includes("--all")) {
  modules = readdirSync(CLIENTS).filter(
    (m) =>
      existsSync(join(CLIENTS, m, "openapi.json")) &&
      !existsSync(join(CLIENTS, m, "src", "index.ts")) // skip hand-built
  );
}

console.log(`Generating TS clients for: ${modules.join(", ")}\n`);

let ok = 0;
let fail = 0;
for (const mod of modules) {
  if (generate(mod)) ok++;
  else fail++;
}

console.log(`\nDone: ${ok} succeeded, ${fail} failed`);
if (fail > 0) process.exit(1);
