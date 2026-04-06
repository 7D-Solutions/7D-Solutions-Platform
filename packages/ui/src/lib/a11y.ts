/**
 * Returns props to visually hide an element while keeping it accessible to
 * screen readers. Equivalent to the common "sr-only" pattern.
 */
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

/**
 * Generates a stable ID for ARIA label associations.
 * Useful for linking form inputs to their labels or descriptions.
 */
export function ariaId(base: string, suffix: string): string {
  return `${base}-${suffix}`;
}

/**
 * Returns the correct aria-invalid value for a field with optional errors.
 * Returns undefined when there are no errors so the attribute is omitted.
 */
export function ariaInvalid(
  hasError: boolean
): React.AriaAttributes["aria-invalid"] {
  return hasError ? "true" : undefined;
}

/**
 * Returns aria-describedby value only when a description ID is present.
 * Avoids setting the attribute to an empty string.
 */
export function ariaDescribedBy(
  ...ids: Array<string | undefined>
): string | undefined {
  const valid = ids.filter(Boolean) as string[];
  return valid.length > 0 ? valid.join(" ") : undefined;
}

// Keep React import for JSX CSSProperties type
import type React from "react";
