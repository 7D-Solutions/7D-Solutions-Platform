import React from "react";
import { cn } from "../../lib/cn.js";

export interface EmptyStateProps extends React.HTMLAttributes<HTMLDivElement> {
  icon?: React.ReactNode;
  title: string;
  description?: string;
  action?: React.ReactNode;
  secondaryAction?: React.ReactNode;
}

/**
 * EmptyState — shown when a list or table has no data.
 *
 * Usage:
 * ```tsx
 * <EmptyState
 *   icon={<Truck className="h-8 w-8" />}
 *   title="No routes yet"
 *   description="Create your first route to get started."
 *   action={<Button>Create route</Button>}
 * />
 * ```
 */
export function EmptyState({
  icon,
  title,
  description,
  action,
  secondaryAction,
  className,
  ...props
}: EmptyStateProps) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-4 rounded-xl border border-dashed border-border py-16 text-center",
        className
      )}
      {...props}
    >
      {icon && (
        <div className="flex h-16 w-16 items-center justify-center rounded-full bg-primary/10 text-primary">
          {icon}
        </div>
      )}
      <div className="flex flex-col gap-1">
        <h3 className="text-base font-semibold text-text-primary">{title}</h3>
        {description && (
          <p className="max-w-xs text-sm text-text-secondary">{description}</p>
        )}
      </div>
      {(action || secondaryAction) && (
        <div className="flex flex-wrap items-center justify-center gap-2">
          {action}
          {secondaryAction}
        </div>
      )}
    </div>
  );
}

/**
 * EmptyStateInline — compact variant for use inside cards or panels.
 */
export function EmptyStateInline({
  icon,
  title,
  description,
  action,
  className,
  ...props
}: Omit<EmptyStateProps, "secondaryAction">) {
  return (
    <div
      className={cn(
        "flex items-start gap-4 rounded-lg border border-dashed border-border p-4",
        className
      )}
      {...props}
    >
      {icon && (
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-primary/10 text-primary">
          {icon}
        </div>
      )}
      <div className="flex flex-1 flex-col gap-1">
        <p className="text-sm font-medium text-text-primary">{title}</p>
        {description && (
          <p className="text-xs text-text-secondary">{description}</p>
        )}
        {action && <div className="pt-1">{action}</div>}
      </div>
    </div>
  );
}
