import React from "react";
import { cn } from "../../lib/cn.js";

export interface BreadcrumbItem {
  label: string;
  href?: string;
  onClick?: (e: React.MouseEvent) => void;
}

export interface BreadcrumbsProps {
  items: BreadcrumbItem[];
  separator?: React.ReactNode;
  className?: string;
}

const DefaultSeparator = () => (
  <svg
    width="12"
    height="12"
    viewBox="0 0 12 12"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.5"
    strokeLinecap="round"
    aria-hidden="true"
  >
    <polyline points="4 2 8 6 4 10" />
  </svg>
);

export function Breadcrumbs({ items, separator, className }: BreadcrumbsProps) {
  return (
    <nav aria-label="Breadcrumb" className={cn("flex", className)}>
      <ol className="flex items-center gap-1 flex-wrap text-sm">
        {items.map((item, index) => {
          const isLast = index === items.length - 1;

          return (
            <li key={index} className="flex items-center gap-1">
              {index > 0 && (
                <span className="text-text-muted shrink-0" aria-hidden="true">
                  {separator ?? <DefaultSeparator />}
                </span>
              )}
              {isLast ? (
                <span
                  aria-current="page"
                  className="text-text-primary font-medium truncate"
                >
                  {item.label}
                </span>
              ) : item.href ? (
                <a
                  href={item.href}
                  onClick={item.onClick}
                  className={cn(
                    "text-text-secondary hover:text-text-primary transition-colors",
                    "hover:underline underline-offset-2 truncate",
                    "focus-visible:outline-none focus-visible:underline"
                  )}
                >
                  {item.label}
                </a>
              ) : (
                <button
                  type="button"
                  onClick={item.onClick}
                  className={cn(
                    "text-text-secondary hover:text-text-primary transition-colors",
                    "hover:underline underline-offset-2 truncate",
                    "focus-visible:outline-none focus-visible:underline"
                  )}
                >
                  {item.label}
                </button>
              )}
            </li>
          );
        })}
      </ol>
    </nav>
  );
}
