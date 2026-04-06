import React from "react";
import { cn } from "../../lib/cn.js";
import { Keys } from "../../lib/keyboard.js";

export interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label?: React.ReactNode;
  disabled?: boolean;
  id?: string;
  className?: string;
  /** aria-label when there is no visible label */
  ariaLabel?: string;
}

export function Switch({
  checked,
  onChange,
  label,
  disabled = false,
  id,
  className,
  ariaLabel,
}: SwitchProps) {
  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === Keys.Space || e.key === Keys.Enter) {
      e.preventDefault();
      if (!disabled) onChange(!checked);
    }
  };

  return (
    <div
      className={cn(
        "inline-flex items-center gap-2",
        disabled && "opacity-60 cursor-not-allowed",
        className
      )}
    >
      <button
        id={id}
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={!label ? ariaLabel : undefined}
        disabled={disabled}
        onClick={() => !disabled && onChange(!checked)}
        onKeyDown={handleKeyDown}
        className={cn(
          "relative inline-flex h-5 w-9 shrink-0 rounded-full border-2 border-transparent",
          "transition-colors duration-200 ease-in-out",
          "focus:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2",
          "disabled:cursor-not-allowed",
          checked ? "bg-primary" : "bg-gray-300"
        )}
      >
        <span
          aria-hidden="true"
          className={cn(
            "pointer-events-none inline-block h-4 w-4 rounded-full bg-white shadow",
            "transition-transform duration-200 ease-in-out",
            checked ? "translate-x-4" : "translate-x-0"
          )}
        />
      </button>
      {label && (
        <span className="text-sm text-text-primary select-none">{label}</span>
      )}
    </div>
  );
}
