import React, { useId, useRef, useState } from "react";
import { cn } from "../lib/cn";

export type TooltipPlacement = "top" | "bottom" | "left" | "right";

export interface TooltipProps {
  content: React.ReactNode;
  placement?: TooltipPlacement;
  children: React.ReactElement;
  className?: string;
  delay?: number;
}

const placementClasses: Record<TooltipPlacement, string> = {
  top: "bottom-full left-1/2 -translate-x-1/2 mb-1.5",
  bottom: "top-full left-1/2 -translate-x-1/2 mt-1.5",
  left: "right-full top-1/2 -translate-y-1/2 mr-1.5",
  right: "left-full top-1/2 -translate-y-1/2 ml-1.5",
};

export function Tooltip({
  content,
  placement = "top",
  children,
  className,
  delay = 400,
}: TooltipProps) {
  const [visible, setVisible] = useState(false);
  const tooltipId = useId();
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const show = () => {
    timerRef.current = setTimeout(() => setVisible(true), delay);
  };

  const hide = () => {
    if (timerRef.current) clearTimeout(timerRef.current);
    setVisible(false);
  };

  const child = React.Children.only(children);
  const childProps = child.props as React.HTMLAttributes<HTMLElement>;
  const childWithProps = React.cloneElement(child, {
    "aria-describedby": visible ? tooltipId : undefined,
    onMouseEnter: (e: React.MouseEvent<HTMLElement>) => { show(); childProps.onMouseEnter?.(e); },
    onMouseLeave: (e: React.MouseEvent<HTMLElement>) => { hide(); childProps.onMouseLeave?.(e); },
    onFocus: (e: React.FocusEvent<HTMLElement>) => { show(); childProps.onFocus?.(e); },
    onBlur: (e: React.FocusEvent<HTMLElement>) => { hide(); childProps.onBlur?.(e); },
  } as React.HTMLAttributes<HTMLElement>);

  return (
    <span className="relative inline-flex">
      {childWithProps}
      {visible && (
        <span
          id={tooltipId}
          role="tooltip"
          className={cn(
            "absolute z-tooltip px-2 py-1 rounded text-xs font-medium",
            "bg-gray-900 text-text-inverse whitespace-nowrap shadow-md",
            "pointer-events-none select-none",
            placementClasses[placement],
            className
          )}
        >
          {content}
        </span>
      )}
    </span>
  );
}
