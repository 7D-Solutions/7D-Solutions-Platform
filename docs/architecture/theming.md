# Theming — Brand Override Contract

**Status:** Active  
**Owner:** Platform Orchestrator  
**Last Updated:** 2026-04-06

---

## How the token system works

All CSS custom properties are declared in `@7d/tokens` and fall into two categories:

| Category | Examples | Brand-overridable? |
|----------|----------|--------------------|
| **Semantic** | `--color-success`, `--color-danger`, `--color-warning`, `--color-info` | **No.** Same across all apps. |
| **Brand primary** | `--color-primary`, `--color-primary-light`, `--color-primary-lighter`, `--color-primary-dark`, `--color-primary-darker` | **Yes.** Each app overrides exactly these five. |

The base palette lives in `packages/tokens/src/tokens.css`. Brand overrides live in `packages/tokens/src/themes/<brand>.css`.

---

## The `data-brand` attribute

Brand themes are activated by setting `data-brand="<name>"` on the root HTML element. The theme file uses an attribute selector to scope its overrides:

```css
/* packages/tokens/src/themes/huberpower.css */
[data-brand="huberpower"] {
  --color-primary: #e05c00;
  --color-primary-light: #f07020;
  --color-primary-lighter: #f89050;
  --color-primary-dark: #b54800;
  --color-primary-darker: #8a3600;
}
```

In the app's `layout.tsx`, set the attribute on `<html>`:

```tsx
<html lang="en" data-brand="huberpower">
```

The scaffold template handles this automatically when you pass `--brand huberpower` to `create-7d-app`.

---

## What each app must do

1. Import `@7d/tokens/tokens.css` — the base token layer. Required.
2. Import `@7d/tokens/themes/<brand>` — your brand override. Required for non-default palettes.
3. Set `data-brand="<brand>"` on `<html>`. Required.
4. Use `bg-primary`, `text-primary`, `border-primary` Tailwind classes (which reference `var(--color-primary)`) — not hardcoded hex.

**In `app/layout.tsx`:**

```tsx
import "@7d/tokens/tokens.css";
import "@7d/tokens/themes/huberpower";
```

---

## What apps must NOT do

- **No CSS module overrides for brand colors.** Do not write `.my-button { background: #e05c00 }`. Use the token.
- **No inline style brand colors.** `style={{ color: "#e05c00" }}` is banned. Use `className="text-primary"`.
- **No overriding semantic tokens.** `--color-success` must stay green on every app. Semantic tokens encode meaning; changing them breaks user expectations.
- **No new theme files created outside `@7d/tokens`.** If a new brand needs primary colors, add a theme to the tokens package — do not create ad-hoc CSS in the app.

---

## Adding a new brand theme

1. Add `packages/tokens/src/themes/<brand>.css` with exactly the five primary overrides.
2. Export it in `packages/tokens/package.json` under `exports`:
   ```json
   "./themes/<brand>": "./src/themes/<brand>.css"
   ```
3. Add the brand slug to the `VALID_BRANDS` array in `packages/create-app/create-7d-app.ts`.
4. Scaffold apps for that brand via `create-7d-app <name> --brand <brand>`.

---

## Available brands

| Brand | Slug | Primary | Use case |
|-------|------|---------|----------|
| TrashTech Pro | `trashtech` | `#1a7340` (forest green) | Waste management |
| HuberPower | `huberpower` | `#e05c00` (industrial orange) | Power generation & utilities |
| RanchOrbit | `ranchorbit` | `#7c5c2e` (saddle brown) | Ranch & livestock management |

---

## Tailwind integration

The `@7d/tokens/preset` wires every token into Tailwind's theme so components can use semantic class names:

```ts
// tailwind.config.ts
import preset from "@7d/tokens/preset";
const config: Config = { presets: [preset] };
```

This means `bg-primary` resolves to `var(--color-primary)`, which at runtime resolves to whatever the active brand theme sets. No Tailwind config changes are needed when switching brands.
