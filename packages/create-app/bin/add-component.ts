#!/usr/bin/env node
/**
 * 7d-add-component — copy UI components from @7d/ui into a vertical project.
 *
 * Reads manifests/importable.json and manifests/copyable.json from the ui package
 * to discover available components, resolves implicit dependencies, copies source
 * files into the target project, and tracks installed components in components.json.
 *
 * Usage: pnpm exec 7d-add-component <component> [<component>...] [--dir <path>] [--dry-run]
 *
 * Examples:
 *   pnpm exec 7d-add-component modal
 *   pnpm exec 7d-add-component data-table button
 *   pnpm exec 7d-add-component DataTable --dir ./apps/trashtech
 */

import { parseArgs } from "node:util";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// ---------------------------------------------------------------------------
// Arg parsing
// ---------------------------------------------------------------------------

const { values: flags, positionals } = parseArgs({
  args: process.argv.slice(2),
  allowPositionals: true,
  options: {
    dir: { type: "string", default: "." },
    "dry-run": { type: "boolean", default: false },
    help: { type: "boolean", short: "h", default: false },
  },
});

const HELP = `
Usage: pnpm exec 7d-add-component <component> [<component>...] [options]

Copies components from the @7d/ui library into your project. Resolves
implicit dependencies automatically.

Options:
  --dir <path>   Target project directory  (default: current directory)
  --dry-run      Print what would be copied without writing files
  --help, -h     Show this help

Examples:
  pnpm exec 7d-add-component modal
  pnpm exec 7d-add-component data-table button badge
  pnpm exec 7d-add-component DataTable --dir ./apps/trashtech

Components: Button, Input, Textarea, Checkbox, RadioGroup, Switch, Label,
  FormField, HelperText, Spinner, Skeleton, SkeletonText, SkeletonCard,
  SkeletonRow, SkeletonTable, SkeletonStat, Separator, Tooltip, Badge,
  EmptyState, EmptyStateInline, GlassCard, PageHeader,
  Modal, Drawer, Toast, ToastContainer, Breadcrumbs, Pagination,
  DataTable, DataTableToolbar, ColumnManager, SearchableSelect, FileUpload,
  useLoadingState, useSearchDebounce, useBeforeUnload, usePagination,
  useColumnManager, useMutationPattern, useQueryInvalidation,
  modalStore, notificationStore, selectionStore, uploadStore
`.trim();

if (flags.help || positionals.length === 0) {
  console.log(HELP);
  process.exit(flags.help ? 0 : 1);
}

const targetDir = resolve(flags.dir as string);
const dryRun = flags["dry-run"] as boolean;

// ---------------------------------------------------------------------------
// Dependency map
// Each key is a component base name (no extension).
// Values are other base names that must be pulled in alongside it.
// ---------------------------------------------------------------------------

const DEPS: Record<string, string[]> = {
  // data-table family — DataTable brings everything it needs
  DataTable: [
    "DataTableToolbar",
    "ColumnManager",
    "RowSelection",
    "useColumnManager",
    "usePagination",
    "selectionStore",
  ],
  DataTableToolbar: ["ColumnManager", "useColumnManager"],
  ColumnManager: ["useColumnManager"],

  // overlays with associated stores
  Modal: ["modalStore"],
  Toast: ["notificationStore"],
  ToastContainer: ["notificationStore", "Toast"],

  // navigation
  Pagination: ["usePagination"],

  // forms
  FileUpload: ["uploadStore", "useLoadingState"],

  // everything else has no extra deps
};

// Aliases: importable export names that live inside a differently-named file,
// plus convenience kebab-case → base-name mappings.
const ALIASES: Record<string, string> = {
  // RowSelection.tsx exports both of these
  SelectAllCheckbox: "RowSelection",
  RowCheckbox: "RowSelection",
  // ToastContainer is exported from Toast.tsx
  ToastContainer: "Toast",
  // GlassCard sub-components all live in GlassCard.tsx
  GlassCardHeader: "GlassCard",
  GlassCardTitle: "GlassCard",
  GlassCardDescription: "GlassCard",
  GlassCardContent: "GlassCard",
  GlassCardFooter: "GlassCard",
  // Skeleton variants all live in Skeleton.tsx
  SkeletonText: "Skeleton",
  SkeletonCard: "Skeleton",
  SkeletonRow: "Skeleton",
  SkeletonTable: "Skeleton",
  SkeletonStat: "Skeleton",
  // EmptyStateInline lives in EmptyState.tsx
  EmptyStateInline: "EmptyState",
};

// ---------------------------------------------------------------------------
// Name normalisation: accept kebab-case, PascalCase, camelCase
// ---------------------------------------------------------------------------

function normaliseName(raw: string): string {
  // Already correct: Button, modalStore, useLoadingState
  if (!raw.includes("-")) return raw;

  const parts = raw.split("-");

  // use-loading-state → useLoadingState
  if (parts[0] === "use") {
    return (
      "use" +
      parts
        .slice(1)
        .map((p) => p.charAt(0).toUpperCase() + p.slice(1))
        .join("")
    );
  }

  // modal-store → modalStore
  if (parts[parts.length - 1] === "store") {
    return (
      parts
        .slice(0, -1)
        .map((p, i) => (i === 0 ? p : p.charAt(0).toUpperCase() + p.slice(1)))
        .join("") + "Store"
    );
  }

  // data-table → DataTable
  return parts.map((p) => p.charAt(0).toUpperCase() + p.slice(1)).join("");
}

// ---------------------------------------------------------------------------
// Locate the @7d/ui package (walk up from targetDir)
// ---------------------------------------------------------------------------

function findUiRoot(start: string): string {
  let dir = start;
  while (true) {
    const candidate = join(dir, "node_modules", "@7d", "ui");
    if (existsSync(join(candidate, "package.json"))) return candidate;
    const parent = dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }

  // Fallback: resolve relative to this script (monorepo dev context)
  const scriptDir = dirname(fileURLToPath(import.meta.url));
  const monorepoCandidate = join(scriptDir, "..", "..", "ui");
  if (existsSync(join(monorepoCandidate, "package.json"))) {
    return monorepoCandidate;
  }

  throw new Error(
    "Cannot find @7d/ui. Run this command from inside a project that has @7d/ui installed."
  );
}

// ---------------------------------------------------------------------------
// Load manifests
// ---------------------------------------------------------------------------

const uiRoot = findUiRoot(targetDir);
const manifestDir = join(uiRoot, "src", "manifests");

const copyableFiles: string[] = JSON.parse(
  readFileSync(join(manifestDir, "copyable.json"), "utf8")
);

// Build base-name → src-relative-path index
// e.g. "Button" → "src/components/primitives/Button.tsx"
const nameToPath = new Map<string, string>();
// Also a lowercase → canonical-name map for case-insensitive fallback
const nameCI = new Map<string, string>(); // lowercase → canonical
for (const filePath of copyableFiles) {
  const base = basename(filePath, filePath.endsWith(".tsx") ? ".tsx" : ".ts");
  nameToPath.set(base, filePath);
  nameCI.set(base.toLowerCase(), base);
}

// ---------------------------------------------------------------------------
// Resolve the full set of files to copy (recursive dep expansion)
// ---------------------------------------------------------------------------

function resolveFiles(names: string[]): Set<string> {
  const collected = new Set<string>();
  const queue = [...names];

  while (queue.length > 0) {
    const raw = queue.shift()!;
    // Resolve: check alias, then exact match, then case-insensitive fallback
    const aliased = ALIASES[raw] ?? raw;
    const name = nameToPath.has(aliased)
      ? aliased
      : (nameCI.get(aliased.toLowerCase()) ?? aliased);

    if (collected.has(name)) continue;

    // Validate
    if (!nameToPath.has(name)) {
      console.error(
        `Error: unknown component "${raw}". Run with --help to see available components.`
      );
      process.exit(1);
    }

    collected.add(name);

    // Enqueue deps
    const deps = DEPS[name] ?? [];
    for (const dep of deps) {
      const resolved = ALIASES[dep] ?? dep;
      if (!collected.has(resolved)) queue.push(dep);
    }
  }

  return collected;
}

// ---------------------------------------------------------------------------
// Target-path logic
// ---------------------------------------------------------------------------

interface ComponentsConfig {
  componentsDir: string;
  hooksDir: string;
  storesDir: string;
  installed: string[];
}

const DEFAULT_CONFIG: Omit<ComponentsConfig, "installed"> = {
  componentsDir: "src/components/ui",
  hooksDir: "src/hooks",
  storesDir: "src/stores",
};

function targetPath(srcRelative: string, cfg: ComponentsConfig): string {
  const file = basename(srcRelative);

  if (srcRelative.startsWith("src/hooks/")) {
    return join(targetDir, cfg.hooksDir, file);
  }
  if (srcRelative.startsWith("src/stores/")) {
    return join(targetDir, cfg.storesDir, file);
  }
  // Component: preserve data-table subdirectory, flatten everything else
  if (srcRelative.startsWith("src/components/data-table/")) {
    return join(targetDir, cfg.componentsDir, "data-table", file);
  }
  return join(targetDir, cfg.componentsDir, file);
}

// ---------------------------------------------------------------------------
// components.json read/write
// ---------------------------------------------------------------------------

const configPath = join(targetDir, "components.json");

function loadConfig(): ComponentsConfig {
  if (existsSync(configPath)) {
    return JSON.parse(readFileSync(configPath, "utf8")) as ComponentsConfig;
  }
  return { ...DEFAULT_CONFIG, installed: [] };
}

function saveConfig(cfg: ComponentsConfig): void {
  writeFileSync(configPath, JSON.stringify(cfg, null, 2) + "\n", "utf8");
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

const inputNames = positionals.map(normaliseName);
const resolvedNames = resolveFiles(inputNames);

const cfg = loadConfig();
const alreadyInstalled = new Set(cfg.installed);

const toInstall: string[] = [];
const toSkip: string[] = [];

for (const name of resolvedNames) {
  if (alreadyInstalled.has(name)) {
    toSkip.push(name);
  } else {
    toInstall.push(name);
  }
}

if (toSkip.length > 0) {
  console.log(`Skipping already-installed: ${toSkip.join(", ")}`);
}

if (toInstall.length === 0) {
  console.log("Nothing new to install.");
  process.exit(0);
}

console.log(`\nInstalling: ${toInstall.join(", ")}${dryRun ? " (dry run)" : ""}\n`);

for (const name of toInstall) {
  const srcRelative = nameToPath.get(name)!;
  const src = join(uiRoot, srcRelative);
  const dest = targetPath(srcRelative, cfg);

  const destDir = dirname(dest);
  // Show destination path relative to targetDir
  const relDest = dest.startsWith(targetDir + "/")
    ? dest.slice(targetDir.length + 1)
    : dest;

  console.log(`  + ${relDest}`);

  if (!dryRun) {
    if (!existsSync(destDir)) mkdirSync(destDir, { recursive: true });
    copyFileSync(src, dest);
  }
}

if (!dryRun) {
  cfg.installed = [...new Set([...cfg.installed, ...toInstall])].sort();
  saveConfig(cfg);
  console.log(`\nUpdated components.json`);
}

console.log();
