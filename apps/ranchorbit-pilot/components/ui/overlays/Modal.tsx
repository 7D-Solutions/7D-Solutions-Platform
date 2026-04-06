import React, { useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { cn } from "../lib/cn";
import { getFocusBoundaries, moveFocus } from "../lib/focus";
import { Keys } from "../lib/keyboard";

export type ModalSize = "sm" | "md" | "lg" | "xl" | "full";

export interface ModalProps {
  open: boolean;
  onClose: () => void;
  title?: React.ReactNode;
  description?: React.ReactNode;
  children: React.ReactNode;
  footer?: React.ReactNode;
  size?: ModalSize;
  closeOnBackdrop?: boolean;
  className?: string;
  "aria-label"?: string;
}

const sizeClasses: Record<ModalSize, string> = {
  sm: "max-w-sm",
  md: "max-w-md",
  lg: "max-w-lg",
  xl: "max-w-xl",
  full: "max-w-full m-4",
};

const CloseIcon = () => (
  <svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <line x1="3" y1="3" x2="13" y2="13" />
    <line x1="13" y1="3" x2="3" y2="13" />
  </svg>
);

export function Modal({
  open,
  onClose,
  title,
  description,
  children,
  footer,
  size = "md",
  closeOnBackdrop = true,
  className,
  "aria-label": ariaLabel,
}: ModalProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const descId = useId();

  useEffect(() => {
    if (!open) return;

    const previouslyFocused = document.activeElement as HTMLElement | null;

    const frame = requestAnimationFrame(() => {
      if (!dialogRef.current) return;
      const { first } = getFocusBoundaries(dialogRef.current);
      if (first) moveFocus(first);
      else moveFocus(dialogRef.current);
    });

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === Keys.Escape) {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key === Keys.Tab && dialogRef.current) {
        const { first, last } = getFocusBoundaries(dialogRef.current);
        if (!first || !last) return;
        if (e.shiftKey) {
          if (document.activeElement === first) { e.preventDefault(); last.focus(); }
        } else {
          if (document.activeElement === last) { e.preventDefault(); first.focus(); }
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
    <div className="fixed inset-0 z-modal-backdrop flex items-center justify-center p-4">
      <div
        className="absolute inset-0 bg-gray-900/60 backdrop-blur-sm"
        aria-hidden="true"
        onClick={closeOnBackdrop ? onClose : undefined}
      />
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={title ? titleId : undefined}
        aria-describedby={description ? descId : undefined}
        aria-label={!title ? ariaLabel : undefined}
        className={cn(
          "relative z-modal w-full bg-bg-primary rounded-lg shadow-xl",
          "flex flex-col max-h-[90vh]",
          sizeClasses[size],
          className
        )}
        onClick={(e) => e.stopPropagation()}
      >
        {(title || description) && (
          <div className="flex items-start justify-between gap-4 p-6 border-b border-border shrink-0">
            <div>
              {title && <h2 id={titleId} className="text-lg font-semibold text-text-primary">{title}</h2>}
              {description && <p id={descId} className="mt-1 text-sm text-text-secondary">{description}</p>}
            </div>
            <button
              type="button"
              aria-label="Close dialog"
              onClick={onClose}
              className={cn("shrink-0 rounded-md p-1 text-text-muted", "hover:bg-gray-100 hover:text-text-primary", "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary")}
            >
              <CloseIcon />
            </button>
          </div>
        )}
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
