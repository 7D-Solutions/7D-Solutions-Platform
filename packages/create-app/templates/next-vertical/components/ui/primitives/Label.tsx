import React from "react";
import { cn } from "../lib/cn";

export interface LabelProps extends React.LabelHTMLAttributes<HTMLLabelElement> {
  required?: boolean;
}

export function Label({ required, className, children, ...rest }: LabelProps) {
  return (
    <label className={cn("block text-sm font-medium text-text-primary", className)} {...rest}>
      {children}
      {required && (
        <span className="ml-0.5 text-danger" aria-hidden="true">*</span>
      )}
    </label>
  );
}
