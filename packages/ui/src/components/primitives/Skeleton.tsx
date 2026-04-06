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
