# HuberPower — Platform Adoption Cookbook

**Bead:** bd-2k9jw  
**Date:** 2026-04-06  
**Status:** Reference guide for new HuberPower vertical teams

---

## What HuberPower is

HuberPower is a power generation and utilities vertical. Its brand theme uses industrial orange (`#e05c00`). The platform token slug is `huberpower`.

---

## Step 1 — Scaffold the app

Run the scaffold CLI from the repo root. Requires Node.js 22+.

```bash
node --experimental-strip-types packages/create-app/create-7d-app.ts \
  huberpower-app \
  --brand huberpower \
  --dir apps/huberpower-app
```

Replace `huberpower-app` with your actual app name (lowercase kebab-case).

The scaffold creates a fully-wired Next.js 15 app in `apps/huberpower-app/` with:
- `@7d/tokens` + HuberPower brand palette applied
- `@7d/platform-client` QueryClient and session wired in `providers.tsx`
- Full foundation component library under `components/ui/`
- Auth pages, TypeScript `@/` alias, Tailwind config

---

## Step 2 — Verify the brand is applied

Open `app/layout.tsx` in the scaffolded app. It should read:

```tsx
import "@7d/tokens/tokens.css";
import "@7d/tokens/themes/huberpower";

// ...

<html lang="en" data-brand="huberpower">
```

The `data-brand="huberpower"` attribute activates the brand override. All `bg-primary` / `text-primary` Tailwind classes will resolve to the industrial orange palette:

| Token | Value |
|-------|-------|
| `--color-primary` | `#e05c00` |
| `--color-primary-light` | `#f07020` |
| `--color-primary-lighter` | `#f89050` |
| `--color-primary-dark` | `#b54800` |
| `--color-primary-darker` | `#8a3600` |

---

## Step 3 — Install dependencies

From the repo root:

```bash
pnpm install
```

This links `@7d/tokens` and `@7d/platform-client` as workspace dependencies.

---

## Step 4 — Configure the API URL

Set the platform API base URL. For local development:

```bash
# apps/huberpower-app/.env.local
NEXT_PUBLIC_API_URL=http://localhost:3001
```

For production, set this as an environment variable in your deployment.

---

## Step 5 — Start the dev server

```bash
cd apps/huberpower-app
pnpm dev
```

The app runs at `http://localhost:3000` with Turbopack enabled by default.

---

## Step 6 — Build your first domain screen

All foundation components are in `components/ui/` — already copied in by the scaffold. Import them with the `@/` alias:

```ts
import { DataTable } from "@/components/ui/data-table/DataTable";
import type { ColumnDef } from "@/components/ui/data-table/DataTable";
import { Badge } from "@/components/ui/primitives/Badge";
import { SearchableSelect } from "@/components/ui/forms/SearchableSelect";
```

### Available foundation components

**Primitives** — `Button`, `Input`, `Textarea`, `Checkbox`, `RadioGroup`, `Switch`, `Label`, `FormField`, `HelperText`, `Spinner`, `Skeleton`, `Separator`, `Tooltip`, `Badge`

**Forms** — `SearchableSelect`, `FileUpload`

**Navigation** — `Breadcrumbs`, `Pagination`

**Overlays** — `Modal`, `Drawer`, `Toast`, `ToastContainer`

**Data** — `DataTable`, `DataTableToolbar`, `ColumnManager`

**Hooks** — `useLoadingState`, `useSearchDebounce`, `useBeforeUnload`, `usePagination`, `useColumnManager`, `useMutationPattern`, `useQueryInvalidation`

**Stores (Zustand)** — `modalStore`, `notificationStore`, `selectionStore`, `uploadStore`

---

## Step 7 — Wire the auth fetcher

Create an API client for your backend. In `lib/api.ts` (or similar):

```ts
import { createAuthFetcher, refreshSession, useSessionStore } from "@7d/platform-client";

export const apiFetch = createAuthFetcher({
  baseUrl: process.env.NEXT_PUBLIC_API_URL!,
  getToken: () => useSessionStore.getState().accessToken,
  onRefreshNeeded: async () => {
    const result = await refreshSession();
    useSessionStore.getState().setTokens(result);
    return result.accessToken;
  },
});
```

Use `apiFetch` in your TanStack Query hooks:

```ts
import { useQuery } from "@tanstack/react-query";
import { apiFetch } from "@/lib/api";

export function useSites() {
  return useQuery({
    queryKey: ["sites"],
    queryFn: () => apiFetch<Site[]>("/v1/sites"),
  });
}
```

---

## Step 8 — Domain screen pattern (DataTable)

```tsx
// app/sites/page.tsx
"use client";
import { DataTable } from "@/components/ui/data-table/DataTable";
import type { ColumnDef } from "@/components/ui/data-table/DataTable";
import { Badge } from "@/components/ui/primitives/Badge";
import { useSites } from "@/hooks/useSites";

type Site = { id: string; name: string; status: "online" | "offline" };

const COLUMNS: ColumnDef<Site>[] = [
  { id: "name", header: "Site Name", cell: (row) => row.name },
  {
    id: "status",
    header: "Status",
    cell: (row) => (
      <Badge variant={row.status === "online" ? "success" : "danger"}>
        {row.status}
      </Badge>
    ),
  },
];

export default function SitesPage() {
  const { data = [] } = useSites();
  return (
    <DataTable
      tableId="sites"
      columns={COLUMNS}
      data={data}
      getRowId={(s) => s.id}
      columnManagerEnabled
    />
  );
}
```

---

## What NOT to do

| Mistake | Correct approach |
|---------|-----------------|
| `style={{ color: "#e05c00" }}` | `className="text-primary"` |
| Overriding `--color-success` or `--color-danger` | Never — semantic tokens are platform-wide |
| Importing UI components from another app's `components/ui/` | Each app owns its copy; import locally via `@/components/ui/` |
| Copying `@7d/platform-client` source files into the app | Import from the package: `@7d/platform-client` |
| Creating a local `tokens.css` with brand colors | Use `@7d/tokens/themes/huberpower` |

---

## Typecheck

```bash
cd apps/huberpower-app
pnpm typecheck
```

Should report zero errors before committing domain screens.

---

## Further reading

- `docs/architecture/theming.md` — brand override contract in full
- `docs/architecture/shared-vs-copied.md` — when to import vs copy
- `docs/architecture/platform-client-boundary.md` — what `@7d/platform-client` provides
- `docs/frontend/PLATFORM-FRONTEND-STANDARDS.md` — full platform standards index
- `docs/migration/ranchorbit.md` — pilot reference (RanchOrbit completed the same flow)
