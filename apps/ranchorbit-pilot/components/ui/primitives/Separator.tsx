import React from "react";
import { cn } from "../lib/cn";

export interface SeparatorProps extends React.HTMLAttributes<HTMLHRElement> {
  orientation?: "horizontal" | "vertical";
}

export function Separator({ orientation = "horizontal", className, ...rest }: SeparatorProps) {
  return (
    <hr
      role="separator"
      aria-orientation={orientation}
      className={cn(
        "border-border-light",
        orientation === "horizontal" ? "w-full border-t" : "h-full border-l self-stretch",
        className
      )}
      {...rest}
    />
  );
}
