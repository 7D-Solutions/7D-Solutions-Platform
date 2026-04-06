const FOCUSABLE_SELECTOR = [
  "a[href]",
  "area[href]",
  "input:not([disabled]):not([type=hidden])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "button:not([disabled])",
  "iframe",
  "object",
  "embed",
  "[contenteditable]",
  '[tabindex]:not([tabindex="-1"])',
].join(", ");

export function getFocusable(container: HTMLElement): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
}

export function getFocusBoundaries(
  container: HTMLElement
): { first: HTMLElement | null; last: HTMLElement | null } {
  const focusable = getFocusable(container);
  return {
    first: focusable[0] ?? null,
    last: focusable[focusable.length - 1] ?? null,
  };
}

export function trapFocus(
  container: HTMLElement
): (e: KeyboardEvent) => void {
  return (e: KeyboardEvent) => {
    if (e.key !== "Tab") return;
    const { first, last } = getFocusBoundaries(container);
    if (!first || !last) return;
    if (e.shiftKey) {
      if (document.activeElement === first) {
        e.preventDefault();
        last.focus();
      }
    } else {
      if (document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  };
}

export function moveFocus(el: HTMLElement): void {
  if (el.tabIndex < 0) {
    el.tabIndex = -1;
  }
  el.focus();
}
