import React from "react";
import { cn } from "../lib/cn";

export interface HelperTextProps extends React.HTMLAttributes<HTMLParagraphElement> {
  error?: boolean;
}

export function HelperText({ error = false, className, children, ...rest }: HelperTextProps) {
  return (
    <p
      className={cn("mt-1 text-xs", error ? "text-danger" : "text-text-muted", className)}
      {...rest}
    >
      {children}
    </p>
  );
}
