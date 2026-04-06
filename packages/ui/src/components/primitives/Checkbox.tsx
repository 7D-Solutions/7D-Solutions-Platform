import React from "react";
import { cn } from "../../lib/cn.js";

export interface CheckboxProps
  extends Omit<React.InputHTMLAttributes<HTMLInputElement>, "type"> {
  label?: React.ReactNode;
  error?: boolean;
}

export const Checkbox = React.forwardRef<HTMLInputElement, CheckboxProps>(
  function Checkbox({ label, error = false, className, id, disabled, ...rest }, ref) {
    return (
      <label
        className={cn(
          "inline-flex items-center gap-2 cursor-pointer select-none",
          disabled && "cursor-not-allowed opacity-60",
          className
        )}
      >
        <input
          ref={ref}
          id={id}
          type="checkbox"
          disabled={disabled}
          aria-invalid={error ? "true" : undefined}
          className={cn(
            "h-4 w-4 rounded border shrink-0",
            "text-primary bg-bg-primary",
            "transition-colors duration-150",
            "focus:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-1",
            "disabled:cursor-not-allowed",
            error ? "border-danger" : "border-border"
          )}
          {...rest}
        />
        {label && (
          <span className={cn("text-sm text-text-primary", error && "text-danger")}>
            {label}
          </span>
        )}
      </label>
    );
  }
);
