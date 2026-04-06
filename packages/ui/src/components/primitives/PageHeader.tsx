import React from "react";
import { cn } from "../../lib/cn.js";

export interface PageHeaderProps extends React.HTMLAttributes<HTMLDivElement> {
  title: string;
  description?: string;
  /** Optional breadcrumb or back-link content */
  eyebrow?: React.ReactNode;
  /** Slot for action buttons (top-right) */
  actions?: React.ReactNode;
}

/**
 * PageHeader — standard page-level title block.
 *
 * Usage:
 * ```tsx
 * <PageHeader
 *   title="Customers"
 *   description="Manage service accounts"
 *   actions={<Button>Add customer</Button>}
 * />
 * ```
 */
export const PageHeader = React.forwardRef<HTMLDivElement, PageHeaderProps>(
  ({ title, description, eyebrow, actions, className, ...props }, ref) => (
    <div
      ref={ref}
      className={cn(
        "flex flex-col gap-1 sm:flex-row sm:items-start sm:justify-between",
        className
      )}
      {...props}
    >
      <div className="flex flex-col gap-1">
        {eyebrow && (
          <div className="text-xs font-medium uppercase tracking-widest text-primary/70">
            {eyebrow}
          </div>
        )}
        <h1 className="text-2xl font-bold tracking-tight text-text-primary sm:text-3xl">
          {title}
        </h1>
        {description && (
          <p className="text-sm text-text-secondary">{description}</p>
        )}
      </div>
      {actions && (
        <div className="flex shrink-0 flex-wrap items-center gap-2 pt-1 sm:pt-0">
          {actions}
        </div>
      )}
    </div>
  )
);
PageHeader.displayName = "PageHeader";
