# Shared vs Copied — Decision Tree

**Status:** Active  
**Owner:** Platform Orchestrator  
**Last Updated:** 2026-04-06

---

When you are building something new, use this decision tree to decide where it belongs.

---

## Decision tree

```
Is this a UI component (renders JSX)?
│
├─ YES → Is it needed by exactly one app?
│        ├─ YES → It belongs in that app's components/ directory. Do not extract it.
│        └─ NO  → Does it need to vary significantly per app (layout, domain labels, etc.)?
│                 ├─ YES → Copy it into the scaffold template (packages/create-app/templates/next-vertical/components/ui/).
│                 │         Each app owns its copy and can diverge.
│                 └─ NO  → Same answer: copy into scaffold template.
│                           (There is no shared UI component library — apps always own their UI files.)
│
└─ NO  → Is it runtime infrastructure (auth, session, HTTP, QueryClient)?
         ├─ YES → It belongs in @7d/platform-client.
         └─ NO  → Is it a design token, color, spacing value, or typography scale?
                  ├─ YES → It belongs in @7d/tokens.
                  └─ NO  → Is it a build/lint/TS config?
                           ├─ YES → It belongs in packages/config/.
                           └─ NO  → It belongs in the app itself.
```

---

## The key rule: UI is always copied

There is no `@7d/ui` package that apps import. UI components are copied into each app at scaffold time and the app owns them from that point. This is intentional:

- Domain screens often need small component tweaks (extra props, layout variations, domain-specific labels).
- Maintaining a versioned shared component library creates coordination overhead that slows down vertical development.
- Apps diverge in acceptable ways without needing upstream approval for every change.

If a fix or improvement is valuable to all verticals, propose it to the scaffold template — but the current state of each live app is not automatically updated.

---

## What belongs in `@7d/platform-client`

Things that would be wrong or broken if each app reimplemented them independently:

- JWT decode and expiry check (`claims.ts`)
- Session state store (Zustand, `session-store.ts`)
- JWT refresh logic (`jwt-refresh.ts`)
- Auth-aware HTTP fetcher (`auth-fetcher.ts`)
- QueryClient factory with platform-standard settings (`query-client.ts`)
- Typed query key helpers (`queryKeys`)

All apps get the same implementation, and security fixes flow to all apps without manual merging.

---

## What belongs in `@7d/tokens`

Things that need to be consistent across apps:

- All CSS custom properties (colors, typography, spacing, shadows, z-index, etc.)
- Tailwind preset that maps those custom properties to Tailwind utilities
- Brand theme CSS files (one per vertical, only overrides primary colors)

---

## What belongs in the scaffold template (`packages/create-app/templates/next-vertical/`)

The starting point for every new vertical app:

- Foundation UI components (`components/ui/`)
- App shell: `layout.tsx`, `providers.tsx`, `globals.css`, `tailwind.config.ts`
- Auth pages
- TypeScript config, ESLint config, Prettier config

These files are **copied** at scaffold time. The app owns them after that.

---

## What belongs in the app only

- Domain screens (`app/<route>/page.tsx`)
- Domain-specific components (e.g. `components/AnimalCard.tsx`, `components/InvoiceLineItem.tsx`)
- Domain API hooks (e.g. `hooks/useAnimals.ts`)
- App-level layout decisions (sidebar, nav structure)
- Playwright E2E tests (`e2e/`)

These never go in shared packages.

---

## Anti-patterns to avoid

| Anti-pattern | Problem | Correct approach |
|---|---|---|
| Hardcoding brand hex values in a component | Breaks theming | Use `var(--color-primary)` or `text-primary` Tailwind class |
| Copying `@7d/platform-client` files into an app | Auth logic forks, security fixes don't propagate | Import from `@7d/platform-client` |
| Creating a local `tokens.css` in an app | Token values diverge from the platform | Import from `@7d/tokens/tokens.css` |
| Importing UI components from another app | Creates hidden coupling between apps | Each app owns its copy; diverge as needed |
| Adding product-specific logic to `@7d/platform-client` | Contaminates shared infrastructure | App-specific logic stays in the app |
