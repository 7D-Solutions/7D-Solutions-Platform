import React from "react";
import { cn } from "../../lib/cn.js";

export type GlassCardVariant = "default" | "elevated" | "subtle";
export type GlassCardPadding = "none" | "sm" | "md" | "lg";

export interface GlassCardProps extends React.HTMLAttributes<HTMLDivElement> {
  variant?: GlassCardVariant;
  padding?: GlassCardPadding;
}

const variantClasses: Record<GlassCardVariant, string> = {
  default:
    "bg-bg-primary/80 border-border hover:bg-bg-primary/90 hover:border-primary/20",
  elevated:
    "bg-bg-primary/90 border-primary/20 shadow-lg hover:shadow-xl hover:border-primary/30",
  subtle: "bg-bg-secondary/50 border-border/50",
};

const paddingClasses: Record<GlassCardPadding, string> = {
  none: "",
  sm: "p-4",
  md: "p-6",
  lg: "p-8",
};

export const GlassCard = React.forwardRef<HTMLDivElement, GlassCardProps>(
  ({ className, variant = "default", padding = "md", ...props }, ref) => (
    <div
      ref={ref}
      className={cn(
        "rounded-xl border backdrop-blur-sm transition-all duration-200",
        variantClasses[variant],
        paddingClasses[padding],
        className
      )}
      {...props}
    />
  )
);
GlassCard.displayName = "GlassCard";

export const GlassCardHeader = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn("flex flex-col gap-1.5", className)}
    {...props}
  />
));
GlassCardHeader.displayName = "GlassCardHeader";

export const GlassCardTitle = React.forwardRef<
  HTMLHeadingElement,
  React.HTMLAttributes<HTMLHeadingElement>
>(({ className, ...props }, ref) => (
  <h3
    ref={ref}
    className={cn(
      "text-lg font-semibold leading-none tracking-tight text-text-primary",
      className
    )}
    {...props}
  />
));
GlassCardTitle.displayName = "GlassCardTitle";

export const GlassCardDescription = React.forwardRef<
  HTMLParagraphElement,
  React.HTMLAttributes<HTMLParagraphElement>
>(({ className, ...props }, ref) => (
  <p
    ref={ref}
    className={cn("text-sm text-text-secondary", className)}
    {...props}
  />
));
GlassCardDescription.displayName = "GlassCardDescription";

export const GlassCardContent = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div ref={ref} className={cn("pt-0", className)} {...props} />
));
GlassCardContent.displayName = "GlassCardContent";

export const GlassCardFooter = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn("flex items-center pt-4", className)}
    {...props}
  />
));
GlassCardFooter.displayName = "GlassCardFooter";
