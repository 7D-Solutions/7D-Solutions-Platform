# `@7d/platform-client` — Package Boundary

**Status:** Active  
**Owner:** Platform Orchestrator  
**Last Updated:** 2026-04-06

---

## What this package is

`@7d/platform-client` is the shared runtime library that every vertical app imports. It handles the parts of a web app that must be identical across all verticals: authentication, session management, and the API client setup.

It is a TypeScript-first package (`main` points to `.ts` source — no build step needed) consumed as a pnpm workspace dependency.

---

## What is inside

| Module | Exports | Purpose |
|--------|---------|---------|
| `error.ts` | `ApiError` | Typed error class for platform API responses |
| `claims.ts` | `AccessClaims`, `decodeClaims`, `isExpired` | JWT decode and expiry check |
| `session-store.ts` | `SessionState`, `useSessionStore` | Zustand store for the active session (user, tenant, token) |
| `jwt-refresh.ts` | `TokenResponse`, `refreshSession` | Refresh-token exchange against the platform auth service |
| `auth-fetcher.ts` | `AuthFetcherOptions`, `AuthFetch`, `createAuthFetcher` | Factory that returns a typed `fetch` wrapper with auth headers and auto-refresh |
| `query-client.ts` | `createQueryClient`, `queryKeys`, `invalidateEntity`, `invalidateEntityDetail`, `invalidateTenantEntity` | TanStack Query client pre-configured with platform retry and staletime settings; typed query key helpers |

---

## What does NOT belong here

`@7d/platform-client` is infrastructure. It must not contain:

- **UI components.** No JSX, no Tailwind, no component files.
- **Domain logic.** No AR, AP, inventory, or any module-specific business rules.
- **App-specific config.** No hardcoded API URLs, tenant IDs, or environment-specific values.
- **Routing.** No Next.js-specific imports (`next/navigation`, `next/link`).
- **Product-specific hooks.** `useAnimals()`, `useInvoices()`, etc. belong in the app.

If something requires knowing the app's domain, it does not belong here.

---

## What belongs in the scaffold instead

The scaffold template (`packages/create-app/templates/next-vertical/`) provides the app shell that wires `@7d/platform-client` into a Next.js app:

| File | Responsibility |
|------|---------------|
| `app/providers.tsx` | Creates the QueryClient (via `createQueryClient`) and mounts the QueryClientProvider |
| `app/layout.tsx` | Root layout; imports tokens and sets `data-brand` |
| `app/(auth)/` | Login/logout pages that call `refreshSession` and write to `useSessionStore` |
| `components/ui/` | All UI — the scaffold provides it as a copied starting point |

The boundary: `@7d/platform-client` gives you the building blocks. The scaffold wires them into the app.

---

## Consuming `@7d/platform-client` in an app

### Add to `package.json`

```json
{
  "dependencies": {
    "@7d/platform-client": "workspace:*"
  }
}
```

### Create an auth-aware fetcher

```ts
import { createAuthFetcher, useSessionStore } from "@7d/platform-client";

// Create once per module (not per render)
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

### Use the QueryClient factory

```ts
// app/providers.tsx
import { createQueryClient } from "@7d/platform-client";
import { useState } from "react";
import { QueryClientProvider } from "@tanstack/react-query";

export function Providers({ children }: { children: React.ReactNode }) {
  const [queryClient] = useState(() => createQueryClient());
  return (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}
```

### Read session state

```ts
import { useSessionStore } from "@7d/platform-client";

function UserMenu() {
  const { user, tenant } = useSessionStore();
  return <span>{user?.name} — {tenant?.name}</span>;
}
```

---

## When to add something to `@7d/platform-client`

Add it here if ALL of the following are true:

1. It is needed by two or more vertical apps.
2. It would be wrong if apps implemented it differently (security, consistency).
3. It contains no UI and no domain logic.
4. It is stable enough that a breaking change requires coordination across apps.

If any condition is false, the code belongs in the app.
