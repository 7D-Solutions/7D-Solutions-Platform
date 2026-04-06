import React from "react";
import { cn } from "../lib/cn";
import { ariaDescribedBy, ariaInvalid } from "../lib/a11y";

export interface TextareaProps extends React.TextareaHTMLAttributes<HTMLTextAreaElement> {
  error?: boolean;
  describedBy?: string;
}

export const Textarea = React.forwardRef<HTMLTextAreaElement, TextareaProps>(
  function Textarea({ error = false, describedBy, className, disabled, ...rest }, ref) {
    return (
      <textarea
        ref={ref}
        disabled={disabled}
        aria-invalid={ariaInvalid(error)}
        aria-describedby={ariaDescribedBy(describedBy)}
        className={cn(
          "block w-full rounded-md border bg-bg-primary text-text-primary text-sm",
          "px-3 py-2 min-h-[80px] resize-y",
          "placeholder:text-text-muted",
          "transition-colors duration-150",
          "focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary",
          "disabled:cursor-not-allowed disabled:bg-bg-secondary disabled:text-text-muted",
          error
            ? "border-danger focus:ring-danger focus:border-danger"
            : "border-border hover:border-border-dark",
          className
        )}
        {...rest}
      />
    );
  }
);
