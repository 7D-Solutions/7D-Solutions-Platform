import React from "react";
import { cn } from "../lib/cn";

export type BadgeVariant = "default" | "primary" | "secondary" | "success" | "warning" | "danger" | "info";
export type BadgeSize = "sm" | "md";

export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  variant?: BadgeVariant;
  size?: BadgeSize;
  dot?: boolean;
}

const variantClasses: Record<BadgeVariant, string> = {
  default: "bg-gray-100 text-gray-700 border border-gray-200",
  primary: "bg-primary/10 text-primary-dark border border-primary/20",
  secondary: "bg-secondary/10 text-secondary-dark border border-secondary/20",
  success: "bg-success/10 text-success-dark border border-success/20",
  warning: "bg-warning/20 text-warning-dark border border-warning/30",
  danger: "bg-danger/10 text-danger-dark border border-danger/20",
  info: "bg-info/10 text-info-dark border border-info/20",
};

const dotColors: Record<BadgeVariant, string> = {
  default: "bg-gray-500",
  primary: "bg-primary",
  secondary: "bg-secondary",
  success: "bg-success",
  warning: "bg-warning-dark",
  danger: "bg-danger",
  info: "bg-info",
};

const sizeClasses: Record<BadgeSize, string> = {
  sm: "px-1.5 py-0.5 text-xs",
  md: "px-2 py-0.5 text-sm",
};

export function Badge({
  variant = "default",
  size = "sm",
  dot = false,
  className,
  children,
  ...rest
}: BadgeProps) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 font-medium rounded-full",
        variantClasses[variant],
        sizeClasses[size],
        className
      )}
      {...rest}
    >
      {dot && (
        <span
          aria-hidden="true"
          className={cn("w-1.5 h-1.5 rounded-full shrink-0", dotColors[variant])}
        />
      )}
      {children}
    </span>
  );
}
