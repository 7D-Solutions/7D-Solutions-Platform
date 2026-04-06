# Frontend Monorepo — Package Structure, Import Rules, Contribution Model

**Status:** Active  
**Owner:** Platform Orchestrator  
**Last Updated:** 2026-04-06

---

## Directory layout

```
7D-Solutions-Platform/
├── apps/
│   ├── docs/                    # Storybook component docs
│   ├── sandbox/                 # Development sandbox
│   ├── tenant-control-plane-ui/ # Platform admin UI
│   └── <your-vertical>/         # Scaffolded vertical apps (e.g. ranchorbit-pilot)
│
└── packages/
    ├── config/                  # Shared ESLint/TS config
    ├── create-app/              # CLI scaffold tool (create-7d-app)
    ├── platform-client/         # Shared runtime: auth, session, QueryClient
    ├── tokens/                  # Design token CSS + Tailwind preset
    └── ui/                      # (reserved — not currently active)
```

---

## The two categories of shared code

| | Imported as a package | Copied into app |
|-|-----------------------|-----------------|
| **What** | `@7d/tokens`, `@7d/platform-client` | `components/ui/` tree |
| **How** | `"@7d/tokens": "workspace:*"` in package.json | `create-7d-app` copies files at scaffold time |
| **Updates** | `pnpm install` picks up changes | Re-scaffold or cherry-pick manually |
| **App ownership** | None — app consumes the API | Full — app owns the files |
| **When to use** | Stable shared infrastructure | UI components apps may need to diverge |

---

## Import rules

### Always import from `@7d/platform-client`

Auth, session state, QueryClient, typed fetch — never copy these files into an app.

```ts
import { createAuthFetcher, useSessionStore, createQueryClient } from "@7d/platform-client";
```

### Always import from `@7d/tokens`

CSS tokens and the Tailwind preset are consumed as a package, never copied.

```ts
// layout.tsx
import "@7d/tokens/tokens.css";
import "@7d/tokens/themes/huberpower";
```

```ts
// tailwind.config.ts
import preset from "@7d/tokens/preset";
```

### Use the copied `components/ui/` tree for UI components

After scaffolding, your app owns `components/ui/`. Import from the local path:

```ts
import { Button } from "@/components/ui/primitives/Button";
import { DataTable } from "@/components/ui/data-table/DataTable";
import { Modal } from "@/components/ui/overlays/Modal";
```

Do not import UI components cross-app (e.g. `from "../../other-app/components/ui/..."`). Each app owns its copy.

---

## What the scaffold gives you

Running `create-7d-app <name> --brand <brand>` produces a complete Next.js 15 app:

```
<name>/
├── app/
│   ├── (auth)/          # Login/logout pages
│   ├── globals.css      # Tailwind directives (thin — tokens come from @7d/tokens)
│   ├── layout.tsx       # Imports tokens, sets data-brand, wraps in Providers
│   ├── page.tsx         # Root redirect
│   └── providers.tsx    # QueryClientProvider wired to @7d/platform-client
├── components/ui/       # Full foundation component library (app-owned copy)
│   ├── data-table/      # DataTable, DataTableToolbar, ColumnManager
│   ├── forms/           # SearchableSelect, FileUpload
│   ├── navigation/      # Breadcrumbs, Pagination
│   ├── overlays/        # Modal, Drawer, Toast, ToastContainer
│   ├── primitives/      # Button, Input, Badge, Checkbox, etc.
│   └── index.ts
├── eslint.config.js
├── next.config.ts
├── package.json         # Depends on @7d/tokens and @7d/platform-client
├── postcss.config.mjs
├── prettier.config.js
├── tailwind.config.ts   # Uses @7d/tokens/preset
└── tsconfig.json        # @/ alias wired
```

---

## Contribution model

### Changing `@7d/tokens` (design tokens)

- Platform Orchestrator owns this package.
- App teams submit a change request mail (see `PLATFORM-FRONTEND-STANDARDS.md`) — they do not edit the package directly.
- Token changes flow to all apps on their next `pnpm install`.

### Changing `@7d/platform-client`

- Platform Orchestrator owns this package.
- Breaking changes (removed exports, changed signatures) require a MAJOR version bump and a migration note in REVISIONS.md.
- App teams update their dependency after the platform commit lands.

### Changing `components/ui/` in your app

- Your app owns its copy. You can modify it freely.
- If you fix a bug or add a variant that should benefit all verticals, propose it upstream: file a bead against the scaffold template in `packages/create-app/templates/next-vertical/components/ui/`.
- There is no automatic sync back — upstream changes do not overwrite your app's copy.

### Adding a new foundation component

1. Build and test it in the scaffold template: `packages/create-app/templates/next-vertical/components/ui/`.
2. Export it from the appropriate `index.ts`.
3. Close the bead — the component is available to all new apps scaffolded after that point.
4. Existing apps pick it up by manually copying the file or re-scaffolding.

---

## pnpm workspace

All packages and apps share a single `pnpm-lock.yaml`. Internal packages use `workspace:*` as their version:

```json
"@7d/tokens": "workspace:*",
"@7d/platform-client": "workspace:*"
```

Run `pnpm install` from the repo root to resolve all workspace links.
