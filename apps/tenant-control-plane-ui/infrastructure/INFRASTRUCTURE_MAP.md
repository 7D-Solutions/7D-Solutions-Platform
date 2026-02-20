# TCP UI — Infrastructure Map

**Last Updated:** 2026-02-20
**Purpose:** Quick-reference guide to all centralized infrastructure in this app.
**Rule:** Read this before writing any UI code. If what you need is listed here, use it — do not re-implement.

---

## Quick Reference

| Need to... | Use this | Import from |
|------------|----------|-------------|
| Save form field values (tab-scoped, persistent) | `useFormStore` | `@/infrastructure/state/useFormStore` |
| Save filter values | `useFilterStore` | `@/infrastructure/state/useFilterStore` |
| Save search term + history | `useSearchStore` | `@/infrastructure/state/useSearchStore` |
| Save file upload progress | `useUploadStore` | `@/infrastructure/state/useUploadStore` |
| Save checkbox / multi-select state | `useSelectionStore` | `@/infrastructure/state/useSelectionStore` |
| Save tab index / step / collapsed sections | `useViewStore` | `@/infrastructure/state/useViewStore` |
| Open a modal | `useTabModal` | `@/infrastructure/state/useTabModal` |
| Track in-app notifications | `useNotificationStore` | `@/infrastructure/state/notificationStore` |
| Show a button | `Button` | `@/components/ui/Button` |
| Show a status badge | `StatusBadge` | `@/components/ui/StatusBadge` |
| Show a modal | `Modal` | `@/components/ui/Modal` |
| Show a table with column management | `DataTable` | `@/components/ui/DataTable` |
| Toggle row/card view | `ViewToggle` | `@/components/ui/ViewToggle` |
| Format a date | `formatDate` | `@/infrastructure/utils/formatters` |
| Format currency | `formatCurrency` | `@/infrastructure/utils/formatters` |
| Format a percentage | `formatPercent` | `@/infrastructure/utils/formatters` |
| Persist column config (backend, cross-device) | `userPreferencesService` | `@/infrastructure/services/userPreferencesService` |
| Standardized API mutation | `useMutationPattern` | `@/infrastructure/hooks/useMutationPattern` |
| Invalidate query cache | `useQueryInvalidation` | `@/infrastructure/hooks/useQueryInvalidation` |
| Pagination state | `usePagination` | `@/infrastructure/hooks/usePagination` |
| Debounced search | `useSearchDebounce` | `@/infrastructure/hooks/useSearchDebounce` |
| Coordinated loading state | `useLoadingState` | `@/infrastructure/hooks/useLoadingState` |
| Column show/hide + reorder | `useColumnManager` | `@/infrastructure/hooks/useColumnManager` |
| Warn on browser close (unsaved changes) | `useBeforeUnload` | `@/infrastructure/hooks/useBeforeUnload` |
| Idle timeout monitoring | `useIdleTimeout` | `@/infrastructure/hooks/useIdleTimeout` |
| Nav badge counts | `useBadgeCounts` | `@/infrastructure/hooks/useBadgeCounts` |
| Backend notifications (TanStack Query) | `useNotificationsQuery` | `@/infrastructure/hooks/useNotificationsQuery` |

---

## State Management (Zustand Stores)

### `tabStore` — Tab list + active tab
- **File:** `infrastructure/state/tabStore.ts`
- **Persisted:** localStorage (`tcp-tab-storage`)
- **Usage:**
  ```typescript
  const { openTab, closeTab } = useTabActions();
  const activeTabId = useActiveTabId();
  ```

### `modalStore` + `useTabModal` — Modal management
- **File:** `infrastructure/state/modalStore.ts`, `infrastructure/state/useTabModal.ts`
- **Persisted:** No (in-memory only)
- **Rule:** Always use `useTabModal` — never import `useModalStore` directly in feature components
- **Usage:**
  ```typescript
  const { openModal } = useTabModal();
  openModal('SUSPEND_TENANT', 'SUSPEND', { tenantId });
  ```

### `useFormStore` — Form field values
- **File:** `infrastructure/state/useFormStore.ts`
- **Persisted:** localStorage (keyed by formKey + tabId)
- **Usage:**
  ```typescript
  const { formData, updateField, isDirty, resetForm } = useFormStore('tenant-settings', {
    planId: '', connectionId: ''
  });
  ```

### `useFilterStore` — Filter values
- **File:** `infrastructure/state/useFilterStore.ts`
- **Persisted:** localStorage
- **Usage:**
  ```typescript
  const { filters, setFilter, clearFilters, hasActiveFilters } = useFilterStore('tenant-list', {
    status: '', planId: ''
  });
  ```

### `useSearchStore` — Search term + history
- **File:** `infrastructure/state/useSearchStore.ts`
- **Persisted:** localStorage
- **Usage:**
  ```typescript
  const { searchTerm, setSearchTerm, recentSearches } = useSearchStore('tenant-list');
  ```

### `useUploadStore` — File upload progress
- **File:** `infrastructure/state/useUploadStore.ts`
- **Persisted:** localStorage
- **Usage:**
  ```typescript
  const { files, setFile, updateProgress, markAsUploaded } = useUploadStore('logo-upload');
  ```

### `useSelectionStore` — Checkbox / multi-select
- **File:** `infrastructure/state/useSelectionStore.ts`
- **Persisted:** localStorage
- **Usage:**
  ```typescript
  const { selectedItems, toggleItem, selectAll, selectedCount } = useSelectionStore('invoice-list');
  ```

### `useViewStore` — Active tab, wizard step, collapsed sections
- **File:** `infrastructure/state/useViewStore.ts`
- **Persisted:** localStorage
- **Usage:**
  ```typescript
  const { state, setState } = useViewStore('tenant-detail', { activeTab: 0 });
  ```

### `notificationStore` — In-app notifications
- **File:** `infrastructure/state/notificationStore.ts`
- **Persisted:** No (in-memory only)
- **Usage:**
  ```typescript
  const { addNotification } = useNotificationActions();
  addNotification({ severity: 'error', title: 'Sync failed', message: '...' });
  ```

---

## Standard Hooks

### `useMutationPattern` — API mutations
- **File:** `infrastructure/hooks/useMutationPattern.ts`
- **Usage:** Wrap every write operation — handles loading, errors, toasts, and cache invalidation.

### `useQueryInvalidation` — Cache invalidation
- **File:** `infrastructure/hooks/useQueryInvalidation.ts`
- **Rule:** Never call `queryClient.invalidateQueries()` without a key.

### `usePagination` — Pagination state
- **File:** `infrastructure/hooks/usePagination.ts`
- **Contract:** `{ page, pageSize, totalPages, goToPage, nextPage, prevPage, ... }`

### `useSearchDebounce` — Debounced search
- **File:** `infrastructure/hooks/useSearchDebounce.ts`
- **Rule:** Never use raw `setTimeout` for debounce.

### `useLoadingState` — Coordinated loading
- **File:** `infrastructure/hooks/useLoadingState.ts`
- **Usage:** Wrap async operations with `withLoading(fn)`.

### `useColumnManager` — Column visibility + order
- **File:** `infrastructure/hooks/useColumnManager.ts`
- **Usage:** Pass to `DataTable` as the `columnManager` prop.

### `useBeforeUnload` — Unsaved changes warning
- **File:** `infrastructure/hooks/useBeforeUnload.ts`
- **Note:** Called automatically by `useFormStore`.

### `useIdleTimeout` — 30-minute idle logout
- **File:** `infrastructure/hooks/useIdleTimeout.ts`
- **Usage:** Mount once in the app shell layout.

### `useBadgeCounts` — Nav badge counts
- **File:** `infrastructure/hooks/useBadgeCounts.ts`
- **Usage:** Used by the left nav component to show unread/pending counts.

### `useNotificationsQuery` — Backend-persisted notifications
- **File:** `infrastructure/hooks/useNotificationsQuery.ts`
- **Usage:** TanStack Query hook that fetches from BFF `/api/notifications` with 30s polling. Returns `{ notifications, unreadCount, markAsRead, markAllAsRead }`. Used by `NotificationCenter` to merge with local store.

---

## Services

### `userPreferencesService` — Backend-persisted preferences
- **File:** `infrastructure/services/userPreferencesService.ts`
- **Usage:** Column configs are saved here (not localStorage). Syncs across devices.

---

## Utilities

### `formatters.ts` — Display formatting
- **File:** `infrastructure/utils/formatters.ts`
- **Functions:** `formatDate`, `formatDateTime`, `formatCurrency`, `formatPercent`, `formatNumber`
- **Rule:** Never format dates or currencies inline in components.

---

## UI Components (`components/ui/`)

All components import from `@/components/ui/` (via `index.ts`).

| Component | Purpose |
|-----------|---------|
| `Button` | All buttons — primary/secondary/danger/ghost/outline, all sizes, double-click protection |
| `StatusBadge` | Tenant/subscription/invoice status — colored badges, no drill-down needed |
| `Modal` | Modals — no backdrop close, Escape closes, portal rendering |
| `ViewToggle` | Row/card toggle — preference persisted per user per table |
| `DataTable` | Data tables with column manager built in |
| `FormInput` | Text input with label, error, required indicator |
| `NumericFormInput` | Number input with formatting |
| `FormSelect` | Dropdown select |
| `FormTextarea` | Multi-line text |
| `FormCheckbox` | Checkbox with label |
| `Checkbox` | Simple checkbox (for tables, no label) |
| `FormRadio` | Radio button group |
| `SearchableSelect` | Searchable dropdown |
| `DateRangePicker` | Date range input |
| `FileInput` | File upload input |
| `NotificationCenter` | Bell icon + notification dropdown |
| `NotificationItem` | Single notification row |
| `IdleWarningModal` | "You'll be logged out in N minutes" countdown modal |

---

## Auth (Server-Side)

| File | Purpose |
|------|---------|
| `middleware.ts` | Protects `/app/**` — requires valid JWT + platform_admin role |
| `lib/server/auth.ts` | `guardPlatformAdmin()` — BFF route guard. Use at top of every `/api/**` handler |
| `lib/constants.ts` | `AUTH_COOKIE_NAME`, `REQUIRED_ROLE`, and all other named constants |

---

## Update Protocol

When adding new infrastructure:
1. Create the file in the appropriate directory
2. Add an entry to this map
3. Update `components/ui/index.ts` if adding a component
