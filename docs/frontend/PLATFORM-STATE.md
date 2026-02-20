# Platform Frontend — State Management, Hooks & Enforcement

> **Who reads this:** Any agent writing state logic, API calls, mutations, pagination, or search.
> **What it covers:** All Zustand stores, standard hook patterns, and the ESLint rules that enforce correct usage.
> **Rule:** If your state falls into one of the categories below, you must use the corresponding store. No exceptions.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Extracted from PLATFORM-FRONTEND-STANDARDS.md rev 1.8. Zustand stores, standard hooks (useMutationPattern, useQueryInvalidation, usePagination, useSearchDebounce, useLoadingState), ESLint enforcement rules. Decision Log populated from master. |

---

## State Management — Zustand Stores

All UI state that must survive a tab switch lives in a Zustand store. Never use `useState` for persistent state — the ESLint rules below will catch it and fail the build.

All stores are **tab-scoped**: each browser tab has its own isolated instance of every store. Switching tabs switches context completely without losing data in either tab.

Reference implementations: `docs/reference/fireproof/src/infrastructure/state/`

### Store Reference

| Store | File | Purpose | Persisted |
|-------|------|---------|-----------|
| `tabStore` | `state/tabStore.ts` | Tab list, active tab ID, split view state | localStorage |
| `modalStore` | `state/modalStore.ts` | Which modal is open, with what data | In-memory only |
| `useFormStore` | `state/useFormStore.ts` | Form field values and dirty state | localStorage |
| `useFilterStore` | `state/useFilterStore.ts` | Filter values + active filter detection | localStorage |
| `useSearchStore` | `state/useSearchStore.ts` | Search term + recent search history | localStorage |
| `useUploadStore` | `state/useUploadStore.ts` | File upload metadata and progress | localStorage |
| `useSelectionStore` | `state/useSelectionStore.ts` | Checkbox / multi-select state | localStorage |
| `useViewStore` | `state/useViewStore.ts` | Active tab index, current step, collapsed sections | localStorage |
| `notificationStore` | `state/notificationStore.ts` | In-app notification list | In-memory only |

### Usage Examples

```typescript
// Form state — survives tab switch, tracks isDirty
const { formData, updateField, isDirty, resetForm } = useFormStore('tenant-settings', {
  planId: '',
  connectionId: ''
});

// Filter state — persists active filters, detects hasActiveFilters
const { filters, setFilter, clearFilters, hasActiveFilters } = useFilterStore('tenant-list', {
  status: '',
  planId: ''
});

// Search state — includes recent search history
const { searchTerm, setSearchTerm, recentSearches } = useSearchStore('tenant-list');

// Selection state — multi-select with selectAll support
const { selectedItems, toggleItem, selectAll, selectedCount } = useSelectionStore('invoice-list');

// View state — active tab, current wizard step, collapsed sections
const { activeTab, setActiveTab } = useViewStore('tenant-detail', { activeTab: 0 });

// Modal — always through useTabModal, never raw useState
const { openModal } = useTabModal();
openModal('SUSPEND_TENANT', { tenantId: tenant.id });
```

### User Preferences — Backend-Persisted

Column configurations are saved to the backend API via `userPreferencesService` — not just localStorage. This ensures staff see their custom column layout on any device.

File: `infrastructure/services/userPreferencesService.ts`
Reference: `docs/reference/fireproof/src/infrastructure/services/userPreferencesService.ts`

---

## Standard Hooks

Every TanStack Query app needs these hooks. If not standardized, each app builds slightly different versions that diverge over time. Each app implements these locally — the **signature and behavior contract** is what is standardized, not the package.

Reference implementations: `docs/reference/fireproof/src/infrastructure/hooks/`

### useMutationPattern

Standardized API mutations with consistent loading state and error surface.

**Contract:**
- Accepts a `mutationFn` (the API call)
- Returns `{ mutate, isPending, error }`
- Error is always surfaced to the caller — never swallowed silently
- Loading state is auto-managed — caller does not need to track it manually
- `onSuccess` callback handles query invalidation and toast notification

```typescript
const { mutate: suspendTenant, isPending } = useMutationPattern({
  mutationFn: (tenantId: string) => api.tenants.suspend(tenantId),
  onSuccess: () => {
    invalidate(['tenant', tenantId]);
    toast.success('Tenant suspended');
  },
  onError: (error) => toast.error(error.message)
});
```

### useQueryInvalidation

Standardized cache invalidation after mutations. Never invalidate all queries with a wildcard — be explicit about what to invalidate.

```typescript
const { invalidate } = useQueryInvalidation();

// Correct — explicit keys
invalidate(['tenant', tenantId]);
invalidate(['tenant-list']);

// Never
queryClient.invalidateQueries(); // invalidates everything — never do this
```

### usePagination

Centralized pagination state and navigation.

**Contract:**
- Returns `{ page, pageSize, totalCount, totalPages, goToPage, nextPage, prevPage, hasNextPage, hasPrevPage }`
- Page is **1-indexed** (page 1 is the first page)
- Default `pageSize` is read from `lib/constants.ts` → `PAGINATION_DEFAULT_PAGE_SIZE`

```typescript
const pagination = usePagination({ totalCount: data?.total ?? 0 });

// Use in query
const { data } = useQuery({
  queryKey: ['tenants', { page: pagination.page, pageSize: pagination.pageSize }],
  queryFn: () => api.tenants.list({ page: pagination.page, pageSize: pagination.pageSize })
});
```

### useSearchDebounce

Debounced search input. Prevents an API call on every keystroke.

**Contract:**
- Accepts `value` (the raw input value) and optional `delay` in ms (default: 300ms)
- Returns the debounced value — use this for query keys, not the raw input
- Delay is configurable per usage — search-as-you-type filtering may use 150ms; heavy queries may use 500ms

```typescript
const [searchInput, setSearchInput] = useState('');
const debouncedSearch = useSearchDebounce(searchInput, 300);

// Use debouncedSearch in query keys — not searchInput
const { data } = useQuery({
  queryKey: ['tenants', { search: debouncedSearch }],
  queryFn: () => api.tenants.list({ search: debouncedSearch })
});
```

**Rule:** Never use raw `setTimeout` for debounce. Never debounce inside a query key directly.

### useLoadingState

Coordinates loading state across multiple concurrent operations. Prevents multiple spinners competing.

**Contract:**
- Returns `{ isLoading, setLoading, withLoading }`
- `withLoading(fn)` — wraps an async function, sets loading true for its duration
- Multiple calls are tracked — `isLoading` remains true until all concurrent operations complete

```typescript
const { isLoading, withLoading } = useLoadingState();

const handleExport = () => withLoading(async () => {
  await api.billing.exportInvoices(tenantId);
  toast.success('Export ready');
});
```

---

## ESLint Enforcement

Custom ESLint rules are active from day one. Violations **fail the build**. No `// eslint-disable` comments without a documented justification that must be approved by the platform orchestrator.

Reference implementation: `docs/reference/fireproof/eslint-local-rules/`

| Rule | What it prevents |
|------|----------------|
| `no-raw-button` | Raw `<button>` elements — import `Button` from `components/ui/` |
| `no-local-modal-state` | `useState` for modal open/close — use `useTabModal` from `modalStore` |
| `no-local-form-state` | `useState` for form fields — use `useFormStore` |
| `no-local-filter-state` | `useState` for filters — use `useFilterStore` |
| `no-local-search-state` | `useState` for search — use `useSearchStore` |
| `no-local-upload-state` | `useState` for file uploads — use `useUploadStore` |
| `no-local-selection-state` | `useState` for checkbox selections — use `useSelectionStore` |
| `no-local-view-state` | `useState` for tab index / step / collapsed state — use `useViewStore` |
| `no-browser-notifications` | `new Notification(...)` or `Notification.requestPermission()` — use platform notification system |

**What these rules do NOT cover (rely on code review):**
- Using the wrong store for a given purpose
- Skipping `useQueryInvalidation` after a mutation
- Hardcoding magic numbers instead of named constants

---

## Open Questions

Do not create beads until these are resolved.

| # | Question | Status |
|---|----------|--------|
| — | No open questions at this time. | — |

---

## Decision Log

Decisions specific to state management and enforcement. Do not re-open without an explicit user directive.

| Date | Decision | Rationale (includes what was NOT chosen) | Decided By |
|------|----------|------------------------------------------|-----------|
| 2026-02-20 | ESLint rules enforced from day one — violations fail build, no override comments without justification | Consistency by tooling not discipline. If rules can be bypassed in emergencies, they will be bypassed routinely. Rejected: lint warnings (ignored), convention-based standards (drift over time). | Platform Orchestrator |
| 2026-02-20 | All persistent UI state in Zustand stores (tab-scoped) — no ad-hoc useState | State survives tab switches without data loss. ESLint rules make violations a build failure from day one. Rejected: component-local useState for persistent state (breaks on tab switch, loses data). | Platform Orchestrator |
| 2026-02-20 | Standard hooks standardized as pattern docs — each app implements locally, no shared package | Every TanStack Query app needs these patterns. Pattern docs prevent divergence without shared package versioning overhead. Rejected: shared npm package (versioning overhead, monorepo coupling, blocks independent deployments). | Platform Orchestrator + TrashTech Orchestrator |
| 2026-02-20 | Column configurations persisted to backend API (cross-device) via userPreferencesService | Staff see their column layout on any device. Rejected: localStorage-only (lost on device switch or browser clear). | Platform Orchestrator |

---

> See `docs/frontend/DOC-REVISION-STANDARDS.md` for the standards governing how this document is maintained.
