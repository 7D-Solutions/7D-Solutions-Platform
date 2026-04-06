/** Selector matching all natively focusable elements. */
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

/**
 * Returns all focusable descendants of a container element.
 */
export function getFocusable(container: HTMLElement): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
}

/**
 * Returns the first and last focusable elements inside a container.
 * Useful for implementing focus traps in modals and dialogs.
 */
export function getFocusBoundaries(
  container: HTMLElement
): { first: HTMLElement | null; last: HTMLElement | null } {
  const focusable = getFocusable(container);
  return {
    first: focusable[0] ?? null,
    last: focusable[focusable.length - 1] ?? null,
  };
}

/**
 * Traps Tab/Shift+Tab focus within a container.
 * Returns an event handler to attach to the container's onKeyDown.
 */
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

/**
 * Moves focus to the given element, falling back to making it temporarily
 * focusable if it has no tabindex.
 */
export function moveFocus(el: HTMLElement): void {
  if (el.tabIndex < 0) {
    el.tabIndex = -1;
  }
  el.focus();
}
