/** Key names used across the component library. */
export const Keys = {
  Enter: "Enter",
  Space: " ",
  Escape: "Escape",
  Tab: "Tab",
  ArrowUp: "ArrowUp",
  ArrowDown: "ArrowDown",
  ArrowLeft: "ArrowLeft",
  ArrowRight: "ArrowRight",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  Backspace: "Backspace",
  Delete: "Delete",
} as const;

export type Key = (typeof Keys)[keyof typeof Keys];

/**
 * Returns true when the keyboard event matches one of the provided keys.
 */
export function isKey(e: KeyboardEvent | React.KeyboardEvent, ...keys: Key[]): boolean {
  return keys.includes(e.key as Key);
}

/**
 * Calls handler only when the event key matches one of the provided keys.
 * Optionally prevents default browser behavior.
 */
export function onKey(
  keys: Key | Key[],
  handler: (e: KeyboardEvent | React.KeyboardEvent) => void,
  options: { preventDefault?: boolean } = {}
): (e: KeyboardEvent | React.KeyboardEvent) => void {
  const keyList = Array.isArray(keys) ? keys : [keys];
  return (e) => {
    if (keyList.includes(e.key as Key)) {
      if (options.preventDefault) e.preventDefault();
      handler(e);
    }
  };
}

/**
 * Returns true when the event was triggered by Enter or Space.
 * Useful for making non-button elements behave as buttons.
 */
export function isActivationKey(e: KeyboardEvent | React.KeyboardEvent): boolean {
  return isKey(e, Keys.Enter, Keys.Space);
}

import type React from "react";
