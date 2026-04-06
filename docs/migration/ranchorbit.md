# RanchOrbit Greenfield Pilot

**Bead:** bd-xz96b  
**Date:** 2026-04-06  
**Status:** Pilot complete — scaffolded and domain screens built

---

## What was validated

This pilot proves the full **zero-to-domain-screens** flow using the 7D platform scaffold tool on a brand-new vertical (no existing codebase to migrate).

### Screens built

| Route | Screen | Foundation components used |
|---|---|---|
| `/dashboard` | Dashboard — herd stats + needs-attention panel | `Badge`, `Separator`, Link |
| `/animals` | Animal list — filterable, sortable table | `DataTable`, `SearchableSelect`, `Badge`, `DataTableToolbar` |
| `/animals/[id]` | Animal detail — identity + status + notes | `Breadcrumbs`, `Badge`, `Button`, `Separator` |

---

## Scaffold command

```bash
node --experimental-strip-types packages/create-app/create-7d-app.ts \
  ranchorbit-pilot \
  --brand ranchorbit \
  --dir apps/ranchorbit-pilot
```

This produces a fully-wired Next.js 15 app with:
- `@7d/tokens` + ranchorbit brand palette applied
- `@7d/platform-client` QueryClient/session wired in `providers.tsx`
- Full foundation component library under `components/ui/`
- Auth pages, tsconfig `@/` alias, Tailwind config — all ready

---

## Foundation component inventory (what the scaffold gives you)

### Primitives (import from `@/components/ui/primitives/...`)
`Button`, `Input`, `Textarea`, `Checkbox`, `RadioGroup`, `Switch`, `Label`, `FormField`, `HelperText`, `Spinner`, `Skeleton`, `Separator`, `Tooltip`, `Badge`

### Forms
`SearchableSelect`, `FileUpload`

### Navigation
`Breadcrumbs`, `Pagination`

### Overlays
`Modal`, `Drawer`, `Toast` + `ToastContainer`

### Data
`DataTable` (sorting, column manager, selection, search), `DataTableToolbar`, `ColumnManager`

### Hooks
`useLoadingState`, `useSearchDebounce`, `useBeforeUnload`, `usePagination`, `useColumnManager`, `useMutationPattern`, `useQueryInvalidation`

### Stores (Zustand)
`modalStore`, `notificationStore`, `selectionStore`, `uploadStore`

---

## Pattern: domain screen with DataTable

```tsx
// Minimal animal list using DataTable + filters
import { DataTable } from "@/components/ui/data-table/DataTable";
import type { ColumnDef } from "@/components/ui/data-table/DataTable";
import { SearchableSelect } from "@/components/ui/forms/SearchableSelect";

const COLUMNS: ColumnDef<Animal>[] = [
  { id: "tag", header: "Tag", cell: (row) => row.tagNumber },
  { id: "status", header: "Status", cell: (row) => <StatusBadge status={row.status} /> },
];

export default function AnimalsPage() {
  return (
    <DataTable
      tableId="animals"
      columns={COLUMNS}
      data={data}
      getRowId={(a) => a.id}
      onRowClick={(a) => router.push(`/animals/${a.id}`)}
      searchValue={search}
      onSearchChange={setSearch}
      columnManagerEnabled
    />
  );
}
```

---

## Bug fixed during pilot

**Template export mismatch:** `components/ui/forms/index.ts` and `components/ui/index.ts` exported `SearchableSelectOption` but the actual type in `SearchableSelect.tsx` is `SelectOption`. Fixed in both the scaffolded app and the template source.

---

## Ranchorbit brand tokens

The `ranchorbit` theme overrides only brand-primary with earthy Western colors:

```css
--color-primary:        #7c5c2e;  /* saddle brown */
--color-primary-light:  #9b7640;
--color-primary-lighter:#c09a60;
--color-primary-dark:   #624722;
--color-primary-darker: #4a3318;
```

All semantic tokens (`--color-success`, `--color-danger`, etc.) are unchanged from the platform default.

---

## Phase 3 exit gate: PASSED

- Greenfield vertical scaffolded via CLI ✓
- Brand theme applied (`data-brand="ranchorbit"`) ✓
- Domain screens built using foundation components (no one-off styles) ✓
- Typecheck passes clean ✓
- No copy-paste of shadcn components — all from `components/ui/` ✓
