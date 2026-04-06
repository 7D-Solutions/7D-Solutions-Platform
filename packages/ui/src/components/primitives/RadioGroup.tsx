import React, { useId } from "react";
import { cn } from "../../lib/cn.js";

export interface RadioOption {
  value: string;
  label: React.ReactNode;
  disabled?: boolean;
}

export interface RadioGroupProps {
  name: string;
  options: RadioOption[];
  value?: string;
  onChange?: (value: string) => void;
  disabled?: boolean;
  error?: boolean;
  /** Screen-reader legend for the group */
  legend?: string;
  orientation?: "vertical" | "horizontal";
  className?: string;
}

export function RadioGroup({
  name,
  options,
  value,
  onChange,
  disabled = false,
  error = false,
  legend,
  orientation = "vertical",
  className,
}: RadioGroupProps) {
  const baseId = useId();

  return (
    <fieldset className={cn("border-0 p-0 m-0", className)}>
      {legend && (
        <legend className="text-sm font-medium text-text-primary mb-1">
          {legend}
        </legend>
      )}
      <div
        role="radiogroup"
        className={cn(
          "flex gap-3",
          orientation === "vertical" ? "flex-col" : "flex-row flex-wrap"
        )}
      >
        {options.map((opt) => {
          const optId = `${baseId}-${opt.value}`;
          const isDisabled = disabled || opt.disabled;
          return (
            <label
              key={opt.value}
              htmlFor={optId}
              className={cn(
                "inline-flex items-center gap-2 cursor-pointer select-none text-sm",
                isDisabled && "cursor-not-allowed opacity-60"
              )}
            >
              <input
                id={optId}
                type="radio"
                name={name}
                value={opt.value}
                checked={value === opt.value}
                disabled={isDisabled}
                aria-invalid={error ? "true" : undefined}
                onChange={(e) => {
                  if (e.target.checked) onChange?.(opt.value);
                }}
                className={cn(
                  "h-4 w-4 border shrink-0",
                  "text-primary bg-bg-primary",
                  "transition-colors duration-150",
                  "focus:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-1",
                  "disabled:cursor-not-allowed",
                  error ? "border-danger" : "border-border"
                )}
              />
              <span className={cn("text-text-primary", error && "text-danger")}>
                {opt.label}
              </span>
            </label>
          );
        })}
      </div>
    </fieldset>
  );
}
