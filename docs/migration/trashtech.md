# TrashTech → Platform Library Adoption

**Bead:** bd-aue04  
**Date:** 2026-04-06  
**Status:** Pilot complete — components extracted, adoption steps documented

---

## Component Mapping

### Use @7d/ui directly (import, don't copy)

These TrashTech components are superseded by platform equivalents. Remove the local copy and import from `@7d/ui`.

| TrashTech file | Platform export | Notes |
|---|---|---|
| `components/ui/button.tsx` | `Button` | Platform adds `loading`, `doubleClickProtection`, `leftIcon/rightIcon` |
| `components/ui/skeleton.tsx` | `Skeleton`, `SkeletonText`, `SkeletonCard`, `SkeletonRow`, `SkeletonTable`, `SkeletonStat` | TrashTech variants are now in the platform |
| `components/ui/toast.tsx` | `Toast`, `ToastContainer` | Platform uses a custom imperative API via `notificationStore`. TrashTech's Sonner-based toast is simpler; keep until a Sonner adapter is added. |

### Extracted to @7d/ui (available as copy-on-pull)

These TrashTech components were extracted into the platform. Use the add-component CLI to copy them into any vertical.

| TrashTech file | Platform export | How to pull |
|---|---|---|
| `components/ui/empty-state.tsx` | `EmptyState`, `EmptyStateInline` | `pnpm exec 7d-add-component EmptyState` |
| `components/ui/glass-card.tsx` | `GlassCard` + sub-components | `pnpm exec 7d-add-component GlassCard` |
| `components/ui/page-header.tsx` | `PageHeader` | `pnpm exec 7d-add-component PageHeader` |

**Token adaptations applied during extraction:**
- `text-foreground` → `text-text-primary`
- `text-muted-foreground` → `text-text-secondary`
- `bg-card/80` → `bg-bg-primary/80`
- `text-destructive` → `text-danger`
- `font-display` → `font-bold` (platform has no dedicated display font)
- `shadow-glow`, `shadow-glow-lg` → `shadow-lg`, `shadow-xl` (TrashTech-specific shadows removed)
- `border-border` — unchanged (same in both systems)
- `bg-primary/10`, `text-primary` — unchanged (both map to `--color-primary`)

### Copy-on-pull only (TrashTech-specific)

These components are too app-specific for the platform library.

| TrashTech file | Reason |
|---|---|
| `components/ui/error-boundary-fallback.tsx` | Depends on `GlassCard`; has a `role` prop tied to TrashTech's admin/driver/customer split. Keep in TrashTech. |

---

## Adopting @7d/tokens

TrashTech currently defines platform semantic colors inline in `globals.css`:

```css
--color-success: #28a745;
--color-warning: #ffc107;
--color-danger: #dc3545;
--color-info: #17a2b8;
```

These are identical to the values in `@7d/tokens`. To remove the duplication:

**1. Add dependency** (`apps/trashtech-pro/package.json`):
```json
"@7d/tokens": "file:../../../7D-Solutions Platform/packages/tokens"
```

**2. Import in globals.css** (before TrashTech's own `:root` block):
```css
@import "@7d/tokens/tokens.css";
```
This provides `--color-success/warning/danger/info` plus all other platform tokens.  
TrashTech's existing shadcn/HSL variables (`--background`, `--primary`, etc.) are unaffected — they use different names and take precedence.

**3. Remove the duplicated block** from `:root`:
```css
/* DELETE these — now sourced from @7d/tokens */
--color-success: #28a745;
--color-warning: #ffc107;
--color-danger: #dc3545;
--color-info: #17a2b8;
```

**4. Tailwind config** — no change required. TrashTech already references `var(--color-success)` etc., which @7d/tokens defines.

### Token system compatibility

TrashTech uses **shadcn/ui HSL tokens** (`--primary: 142 71% 45%`) while the platform uses **hex tokens** (`--color-primary: #1a7340`). These coexist without conflict — different CSS variable names.

The @7d/tokens preset (`@7d/tokens/preset`) is optional for TrashTech. It adds the platform token aliases (`text-text-primary`, `border-border`, etc.) to Tailwind. TrashTech can extend its `tailwind.config.ts` to include it alongside its existing config.

---

## Adopting @7d/config

**1. Add devDependency** (`apps/trashtech-pro/package.json`):
```json
"@7d/config": "file:../../../7D-Solutions Platform/packages/config"
```

**2. Update tsconfig.json** — extend from platform base:
```json
{
  "extends": "@7d/config/tsconfig/nextjs",
  "compilerOptions": {
    "paths": { "@/*": ["./*"] }
  },
  "include": ["next-env.d.ts", "**/*.ts", "**/*.tsx", ".next/types/**/*.ts"],
  "exclude": ["node_modules"]
}
```

**3. ESLint** — replace `eslint-config-next` with `@7d/config/eslint/next`:
```js
// eslint.config.mjs
import nextConfig from "@7d/config/eslint/next";
export default [...nextConfig];
```

---

## Adopting @7d/platform-client

**1. Add dependency** (`apps/trashtech-pro/package.json`):
```json
"@7d/platform-client": "file:../../../7D-Solutions Platform/packages/platform-client"
```

**2. Mapping to TrashTech's existing auth layer:**

| TrashTech | Platform equivalent |
|---|---|
| `lib/auth/session.ts` — JWT decode + role detection | `decodeClaims`, `isExpired` from `@7d/platform-client` |
| `lib/auth/client.ts` — fetch wrapper with Bearer header | `createAuthFetcher` from `@7d/platform-client` |
| `lib/store/` — session zustand store | `useSessionStore` from `@7d/platform-client` |
| `lib/query/` — React Query client setup | `createQueryClient`, `queryKeys` from `@7d/platform-client` |

**Minimal wiring** (`lib/api/client.ts`):
```ts
import { createAuthFetcher } from "@7d/platform-client";

export const authFetch = createAuthFetcher({
  identityAuthBaseUrl: process.env.NEXT_PUBLIC_IDENTITY_AUTH_URL ?? "",
});
```

The existing `lib/auth/session.ts` JWT decode logic can be replaced by `decodeClaims<TrashTechClaims>()`. TrashTech's `SessionClaims` maps directly to `AccessClaims` with `roles` and `perms` arrays.

---

## Install steps (once decisions are made)

```bash
cd apps/trashtech-pro
npm install
# Verify no type errors
npx tsc --noEmit
```

---

## What this proves

- Platform tokens are already used by TrashTech (`--color-success` etc.) — adoption removes duplication, not functionality.
- `@7d/config` produces a compatible tsconfig/ESLint for a Next.js 15 + React 19 project.
- `@7d/platform-client` covers TrashTech's auth and query patterns without rewriting them.
- Extracted components (`EmptyState`, `GlassCard`, `PageHeader`, rich `Skeleton`) work in any vertical using the platform token system.
