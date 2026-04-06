import React from "react";
import { cn } from "../../lib/cn.js";

export type SpinnerSize = "xs" | "sm" | "md" | "lg";

export interface SpinnerProps {
  size?: SpinnerSize;
  className?: string;
  /** Screen-reader label — defaults to "Loading" */
  label?: string;
}

const sizeClasses: Record<SpinnerSize, string> = {
  xs: "w-3 h-3 border",
  sm: "w-4 h-4 border-2",
  md: "w-6 h-6 border-2",
  lg: "w-8 h-8 border-[3px]",
};

export function Spinner({
  size = "md",
  className,
  label = "Loading",
}: SpinnerProps) {
  return (
    <span
      role="status"
      aria-label={label}
      className={cn(
        "inline-block rounded-full border-current border-t-transparent animate-spin",
        sizeClasses[size],
        className
      )}
    />
  );
}
