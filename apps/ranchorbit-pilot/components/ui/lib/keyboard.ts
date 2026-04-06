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

export function isKey(e: KeyboardEvent | React.KeyboardEvent, ...keys: Key[]): boolean {
  return keys.includes(e.key as Key);
}

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

export function isActivationKey(e: KeyboardEvent | React.KeyboardEvent): boolean {
  return isKey(e, Keys.Enter, Keys.Space);
}

import type React from "react";
