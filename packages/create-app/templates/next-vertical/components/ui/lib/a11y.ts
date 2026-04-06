export function srOnly(): React.CSSProperties {
  return {
    position: "absolute",
    width: "1px",
    height: "1px",
    padding: "0",
    margin: "-1px",
    overflow: "hidden",
    clip: "rect(0, 0, 0, 0)",
    whiteSpace: "nowrap",
    borderWidth: "0",
  };
}

export function ariaId(base: string, suffix: string): string {
  return `${base}-${suffix}`;
}

export function ariaInvalid(
  hasError: boolean
): React.AriaAttributes["aria-invalid"] {
  return hasError ? "true" : undefined;
}

export function ariaDescribedBy(
  ...ids: Array<string | undefined>
): string | undefined {
  const valid = ids.filter(Boolean) as string[];
  return valid.length > 0 ? valid.join(" ") : undefined;
}

import type React from "react";
