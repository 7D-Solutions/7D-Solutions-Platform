import React from "react";
import { cn } from "../../lib/cn.js";
import { ariaDescribedBy, ariaInvalid } from "../../lib/a11y.js";

export type InputSize = "sm" | "md" | "lg";

export interface InputProps
  extends Omit<React.InputHTMLAttributes<HTMLInputElement>, "size"> {
  size?: InputSize;
  error?: boolean;
  /** ID of associated helper/error text for aria-describedby */
  describedBy?: string;
}

const sizeClasses: Record<InputSize, string> = {
  sm: "h-8 px-3 text-sm",
  md: "h-9 px-3 text-base",
  lg: "h-11 px-4 text-lg",
};

export const Input = React.forwardRef<HTMLInputElement, InputProps>(
  function Input(
    { size = "md", error = false, describedBy, className, disabled, ...rest },
    ref
  ) {
    return (
      <input
        ref={ref}
        disabled={disabled}
        aria-invalid={ariaInvalid(error)}
        aria-describedby={ariaDescribedBy(describedBy)}
        className={cn(
          "block w-full rounded-md border bg-bg-primary text-text-primary",
          "placeholder:text-text-muted",
          "transition-colors duration-150",
          "focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary",
          "disabled:cursor-not-allowed disabled:bg-bg-secondary disabled:text-text-muted",
          error
            ? "border-danger focus:ring-danger focus:border-danger"
            : "border-border hover:border-border-dark",
          sizeClasses[size],
          className
        )}
        {...rest}
      />
    );
  }
);
