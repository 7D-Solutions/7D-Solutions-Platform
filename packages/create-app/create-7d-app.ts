#!/usr/bin/env node
/**
 * create-7d-app — scaffold a Next.js vertical app from the platform template.
 *
 * Usage: create-7d-app <name> [--brand trashtech|huberpower|ranchorbit] [--api-url <url>] [--dir <path>]
 *
 * Requires Node.js >= 22 (native TypeScript execution via strip-types).
 */

import { parseArgs } from "node:util";
import {
  cpSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
  existsSync,
} from "node:fs";
import { join, resolve, relative, extname, dirname } from "node:path";
import { fileURLToPath } from "node:url";

// ---------------------------------------------------------------------------
// Arg parsing
// ---------------------------------------------------------------------------

const { values: flags, positionals } = parseArgs({
  args: process.argv.slice(2),
  allowPositionals: true,
  options: {
    brand: { type: "string", default: "trashtech" },
    "api-url": { type: "string", default: "http://localhost:3001" },
    dir: { type: "string" },
    help: { type: "boolean", short: "h", default: false },
  },
});

const HELP = `
Usage: create-7d-app <name> [options]

Options:
  --brand <theme>    Brand theme: trashtech | huberpower | ranchorbit  (default: trashtech)
  --api-url <url>    Platform API base URL  (default: http://localhost:3001)
  --dir <path>       Output directory  (default: ./<name>)
  --help, -h         Show this help

Examples:
  create-7d-app my-vertical
  create-7d-app trashtech-app --brand trashtech --api-url https://api.example.com
`.trim();

if (flags.help || positionals.length === 0) {
  console.log(HELP);
  process.exit(flags.help ? 0 : 1);
}

const appName = positionals[0] as string;
const brand = flags.brand as string;
const apiUrl = flags["api-url"] as string;
const outDir = resolve(flags.dir ?? appName);

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

const VALID_NAME = /^[a-z][a-z0-9-]*$/;
const VALID_BRANDS = ["trashtech", "huberpower", "ranchorbit"] as const;

if (!VALID_NAME.test(appName)) {
  console.error(`Error: app name must be lowercase kebab-case (e.g. "my-vertical"), got: "${appName}"`);
  process.exit(1);
}

if (!VALID_BRANDS.includes(brand as (typeof VALID_BRANDS)[number])) {
  console.error(`Error: --brand must be one of ${VALID_BRANDS.join(", ")}, got: "${brand}"`);
  process.exit(1);
}

if (existsSync(outDir)) {
  console.error(`Error: directory already exists: ${outDir}`);
  process.exit(1);
}

// ---------------------------------------------------------------------------
// Substitution helpers
// ---------------------------------------------------------------------------

function toTitleCase(str: string): string {
  return str
    .split("-")
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

const SUBS: Record<string, string> = {
  __APP_NAME__: appName,
  __APP_TITLE__: toTitleCase(appName),
  __BRAND_THEME__: brand,
  __API_URL__: apiUrl,
};

function applySubstitutions(content: string): string {
  let out = content;
  for (const [key, val] of Object.entries(SUBS)) {
    out = out.replaceAll(key, val);
  }
  return out;
}

/** File extensions treated as text (substitutions applied). All others are copied raw. */
const TEXT_EXTS = new Set([
  ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs",
  ".json", ".css", ".md", ".txt", ".html", ".env",
]);

// ---------------------------------------------------------------------------
// Template copy
// ---------------------------------------------------------------------------

function copyTemplate(src: string, dest: string): void {
  mkdirSync(dest, { recursive: true });
  for (const entry of readdirSync(src)) {
    const srcPath = join(src, entry);
    // Strip .tpl suffix from destination name
    const destEntry = entry.endsWith(".tpl") ? entry.slice(0, -4) : entry;
    const destPath = join(dest, destEntry);
    const stat = statSync(srcPath);

    if (stat.isDirectory()) {
      copyTemplate(srcPath, destPath);
    } else {
      const ext = extname(destEntry);
      const isText = TEXT_EXTS.has(ext) || entry.endsWith(".tpl");
      if (isText) {
        const raw = readFileSync(srcPath, "utf8");
        writeFileSync(destPath, applySubstitutions(raw), "utf8");
      } else {
        cpSync(srcPath, destPath);
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Scaffold
// ---------------------------------------------------------------------------

const templateDir = join(
  dirname(fileURLToPath(import.meta.url)),
  "templates",
  "next-vertical",
);

console.log(`\nScaffolding ${appName} in ${outDir}…`);

copyTemplate(templateDir, outDir);

// ---------------------------------------------------------------------------
// Done
// ---------------------------------------------------------------------------

const rel = relative(process.cwd(), outDir);
const cdPath = rel.startsWith(".") ? rel : `./${rel}`;

console.log(`
Done!  Get started:

  cd ${cdPath}
  pnpm install
  pnpm dev

Environment variables are in .env.local — update NEXT_PUBLIC_PLATFORM_API_URL before starting.
`);
