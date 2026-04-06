import React from "react";
import { cn } from "../../lib/cn.js";

export interface SkeletonProps extends React.HTMLAttributes<HTMLDivElement> {
  /** Width — can be any CSS value or a Tailwind class via className */
  width?: string | number;
  /** Height — can be any CSS value or a Tailwind class via className */
  height?: string | number;
  /** Circle variant — sets border-radius to full */
  circle?: boolean;
  /** Disables the shimmer animation */
  static?: boolean;
}

export function Skeleton({
  width,
  height,
  circle = false,
  static: isStatic = false,
  className,
  style,
  ...rest
}: SkeletonProps) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        "bg-gray-200 rounded-md",
        !isStatic && "animate-pulse",
        circle && "rounded-full",
        className
      )}
      style={{
        width: typeof width === "number" ? `${width}px` : width,
        height: typeof height === "number" ? `${height}px` : height,
        ...style,
      }}
      {...rest}
    />
  );
}

/** Single text line skeleton */
export function SkeletonText({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return <Skeleton className={cn("h-4 w-full", className)} {...props} />;
}

/** Card-shaped skeleton with avatar, header lines, and body lines */
export function SkeletonCard({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "rounded-xl border border-border bg-bg-secondary/50 p-6",
        className
      )}
      {...props}
    >
      <div className="flex items-center gap-3 pb-4">
        <Skeleton className="h-10 w-10 rounded-full" />
        <div className="flex-1 space-y-2">
          <Skeleton className="h-4 w-1/3" />
          <Skeleton className="h-3 w-1/2" />
        </div>
      </div>
      <div className="space-y-3">
        <Skeleton className="h-3 w-full" />
        <Skeleton className="h-3 w-5/6" />
        <Skeleton className="h-3 w-4/6" />
      </div>
    </div>
  );
}

/** Table row skeleton */
export function SkeletonRow({
  className,
  cols = 4,
  ...props
}: React.HTMLAttributes<HTMLDivElement> & { cols?: number }) {
  return (
    <div
      className={cn("flex items-center gap-4 py-3", className)}
      {...props}
    >
      {Array.from({ length: cols }).map((_, i) => (
        <Skeleton
          key={i}
          className="h-4"
          style={{ flex: i === 0 ? 2 : 1 }}
        />
      ))}
    </div>
  );
}

/** Full table skeleton: header row + N data rows */
export function SkeletonTable({
  rows = 5,
  cols = 4,
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement> & { rows?: number; cols?: number }) {
  return (
    <div
      className={cn("rounded-xl border border-border bg-bg-secondary/50", className)}
      {...props}
    >
      <div className="flex items-center gap-4 border-b border-border px-6 py-3">
        {Array.from({ length: cols }).map((_, i) => (
          <Skeleton key={i} className="h-3 w-20" />
        ))}
      </div>
      <div className="divide-y divide-border px-6">
        {Array.from({ length: rows }).map((_, i) => (
          <SkeletonRow key={i} cols={cols} />
        ))}
      </div>
    </div>
  );
}

/** Stat card skeleton (single KPI tile) */
export function SkeletonStat({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "rounded-xl border border-border bg-bg-secondary/50 p-6",
        className
      )}
      {...props}
    >
      <Skeleton className="mb-3 h-3 w-24" />
      <Skeleton className="h-8 w-20" />
    </div>
  );
}
