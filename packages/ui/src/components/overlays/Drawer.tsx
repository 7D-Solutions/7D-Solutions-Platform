import React, { useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { cn } from "../../lib/cn.js";
import { getFocusBoundaries, moveFocus } from "../../lib/focus.js";
import { Keys } from "../../lib/keyboard.js";

export type DrawerSide = "left" | "right";
export type DrawerSize = "sm" | "md" | "lg";

export interface DrawerProps {
  open: boolean;
  onClose: () => void;
  title?: React.ReactNode;
  description?: React.ReactNode;
  children: React.ReactNode;
  footer?: React.ReactNode;
  side?: DrawerSide;
  size?: DrawerSize;
  /** Whether clicking the backdrop closes the drawer — default true */
  closeOnBackdrop?: boolean;
  className?: string;
}

const sizeClasses: Record<DrawerSize, string> = {
  sm: "w-72",
  md: "w-96",
  lg: "w-[32rem]",
};

const sideClasses: Record<DrawerSide, string> = {
  left: "left-0",
  right: "right-0",
};

const CloseIcon = () => (
  <svg
    aria-hidden="true"
    width="16"
    height="16"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
  >
    <line x1="3" y1="3" x2="13" y2="13" />
    <line x1="13" y1="3" x2="3" y2="13" />
  </svg>
);

export function Drawer({
  open,
  onClose,
  title,
  description,
  children,
  footer,
  side = "right",
  size = "md",
  closeOnBackdrop = true,
  className,
}: DrawerProps) {
  const panelRef = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const descId = useId();

  useEffect(() => {
    if (!open) return;

    const previouslyFocused = document.activeElement as HTMLElement | null;

    const frame = requestAnimationFrame(() => {
      if (!panelRef.current) return;
      const { first } = getFocusBoundaries(panelRef.current);
      if (first) moveFocus(first);
      else moveFocus(panelRef.current);
    });

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === Keys.Escape) {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key === Keys.Tab && panelRef.current) {
        const { first, last } = getFocusBoundaries(panelRef.current);
        if (!first || !last) return;
        if (e.shiftKey) {
          if (document.activeElement === first) {
            e.preventDefault();
            last.focus();
          }
        } else {
          if (document.activeElement === last) {
            e.preventDefault();
            first.focus();
          }
        }
      }
    };

    document.addEventListener("keydown", handleKeyDown);
    document.body.style.overflow = "hidden";

    return () => {
      cancelAnimationFrame(frame);
      document.removeEventListener("keydown", handleKeyDown);
      document.body.style.overflow = "";
      previouslyFocused?.focus();
    };
  }, [open, onClose]);

  if (!open || typeof document === "undefined") return null;

  return createPortal(
    <div className="fixed inset-0 z-modal-backdrop">
      <div
        className="absolute inset-0 bg-gray-900/50"
        aria-hidden="true"
        onClick={closeOnBackdrop ? onClose : undefined}
      />
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={title ? titleId : undefined}
        aria-describedby={description ? descId : undefined}
        className={cn(
          "absolute z-modal top-0 bottom-0 bg-bg-primary shadow-xl flex flex-col",
          sideClasses[side],
          sizeClasses[size],
          className
        )}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-start justify-between gap-4 p-6 border-b border-border shrink-0">
          <div>
            {title && (
              <h2 id={titleId} className="text-lg font-semibold text-text-primary">
                {title}
              </h2>
            )}
            {description && (
              <p id={descId} className="mt-1 text-sm text-text-secondary">
                {description}
              </p>
            )}
          </div>
          <button
            type="button"
            aria-label="Close panel"
            onClick={onClose}
            className={cn(
              "shrink-0 rounded-md p-1 text-text-muted",
              "hover:bg-gray-100 hover:text-text-primary",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            )}
          >
            <CloseIcon />
          </button>
        </div>
        <div className="flex-1 overflow-y-auto p-6">{children}</div>
        {footer && (
          <div className="flex items-center justify-end gap-3 p-6 border-t border-border shrink-0">
            {footer}
          </div>
        )}
      </div>
    </div>,
    document.body
  );
}
