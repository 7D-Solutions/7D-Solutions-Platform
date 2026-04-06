import React from "react";
import { cn } from "../lib/cn";

export interface SkeletonProps extends React.HTMLAttributes<HTMLDivElement> {
  width?: string | number;
  height?: string | number;
  circle?: boolean;
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
