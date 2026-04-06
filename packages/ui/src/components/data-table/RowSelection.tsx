import React from "react";
import { cn } from "../../lib/cn.js";

interface SelectAllCheckboxProps {
  checked: boolean;
  indeterminate: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  "aria-label"?: string;
}

/** Header checkbox — renders checked, unchecked, or indeterminate. */
export function SelectAllCheckbox({
  checked,
  indeterminate,
  onChange,
  disabled = false,
  "aria-label": ariaLabel = "Select all rows",
}: SelectAllCheckboxProps) {
  const ref = React.useCallback(
    (node: HTMLInputElement | null) => {
      if (node) node.indeterminate = indeterminate;
    },
    [indeterminate]
  );

  return (
    <input
      ref={ref}
      type="checkbox"
      aria-label={ariaLabel}
      checked={checked}
      disabled={disabled}
      onChange={(e) => onChange(e.target.checked)}
      className={cn(
        "h-4 w-4 rounded border shrink-0",
        "text-primary bg-bg-primary",
        "transition-colors duration-150",
        "focus:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-1",
        "disabled:cursor-not-allowed",
        "border-border"
      )}
    />
  );
}

interface RowCheckboxProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  "aria-label"?: string;
}

/** Per-row checkbox. */
export function RowCheckbox({
  checked,
  onChange,
  disabled = false,
  "aria-label": ariaLabel = "Select row",
}: RowCheckboxProps) {
  return (
    <input
      type="checkbox"
      aria-label={ariaLabel}
      checked={checked}
      disabled={disabled}
      onChange={(e) => onChange(e.target.checked)}
      className={cn(
        "h-4 w-4 rounded border shrink-0",
        "text-primary bg-bg-primary",
        "transition-colors duration-150",
        "focus:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-1",
        "disabled:cursor-not-allowed",
        "border-border"
      )}
    />
  );
}
