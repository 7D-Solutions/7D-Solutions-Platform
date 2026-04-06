import React, { useEffect } from "react";
import { createPortal } from "react-dom";
import { cn } from "../../lib/cn.js";

export type ToastVariant = "default" | "success" | "warning" | "danger" | "info";

export type ToastPosition =
  | "top-left"
  | "top-center"
  | "top-right"
  | "bottom-left"
  | "bottom-center"
  | "bottom-right";

export interface ToastProps {
  id: string;
  message: React.ReactNode;
  variant?: ToastVariant;
  /** Duration in ms before auto-dismiss — 0 to disable */
  duration?: number;
  onDismiss: (id: string) => void;
  action?: { label: string; onClick: () => void };
}

export interface ToastContainerProps {
  toasts: ToastProps[];
  position?: ToastPosition;
}

const variantClasses: Record<ToastVariant, string> = {
  default: "bg-gray-900 text-text-inverse border-gray-700",
  success: "bg-success/10 text-success-dark border-success/30",
  warning: "bg-warning/20 text-warning-dark border-warning/30",
  danger: "bg-danger/10 text-danger-dark border-danger/20",
  info: "bg-info/10 text-info-dark border-info/20",
};

const positionClasses: Record<ToastPosition, string> = {
  "top-left": "top-4 left-4 items-start",
  "top-center": "top-4 left-1/2 -translate-x-1/2 items-center",
  "top-right": "top-4 right-4 items-end",
  "bottom-left": "bottom-4 left-4 items-start",
  "bottom-center": "bottom-4 left-1/2 -translate-x-1/2 items-center",
  "bottom-right": "bottom-4 right-4 items-end",
};

const SuccessIcon = () => (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <circle cx="8" cy="8" r="6" />
    <polyline points="5.5 8.5 7 10 10.5 6.5" />
  </svg>
);

const WarningIcon = () => (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <path d="M8 2L14 13H2L8 2z" />
    <line x1="8" y1="7" x2="8" y2="10" />
    <circle cx="8" cy="12" r="0.5" fill="currentColor" />
  </svg>
);

const DangerIcon = () => (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
    <circle cx="8" cy="8" r="6" />
    <line x1="10" y1="6" x2="6" y2="10" />
    <line x1="6" y1="6" x2="10" y2="10" />
  </svg>
);

const InfoIcon = () => (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
    <circle cx="8" cy="8" r="6" />
    <line x1="8" y1="7" x2="8" y2="11" />
    <circle cx="8" cy="5" r="0.5" fill="currentColor" />
  </svg>
);

const DismissIcon = () => (
  <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
    <line x1="2" y1="2" x2="12" y2="12" />
    <line x1="12" y1="2" x2="2" y2="12" />
  </svg>
);

const variantIcons: Record<ToastVariant, React.ReactNode> = {
  default: null,
  success: <SuccessIcon />,
  warning: <WarningIcon />,
  danger: <DangerIcon />,
  info: <InfoIcon />,
};

export function Toast({
  id,
  message,
  variant = "default",
  duration = 5000,
  onDismiss,
  action,
}: ToastProps) {
  useEffect(() => {
    if (!duration) return;
    const timer = setTimeout(() => onDismiss(id), duration);
    return () => clearTimeout(timer);
  }, [id, duration, onDismiss]);

  return (
    <div
      role="alert"
      aria-live="polite"
      className={cn(
        "flex items-start gap-3 w-full max-w-sm p-4 rounded-lg border shadow-md",
        "pointer-events-auto",
        variantClasses[variant]
      )}
    >
      {variantIcons[variant] && (
        <span className="shrink-0 mt-0.5">{variantIcons[variant]}</span>
      )}
      <div className="flex-1 min-w-0 text-sm font-medium">{message}</div>
      {action && (
        <button
          type="button"
          onClick={action.onClick}
          className="shrink-0 text-xs font-semibold underline underline-offset-2 hover:opacity-80 focus-visible:outline-none"
        >
          {action.label}
        </button>
      )}
      <button
        type="button"
        aria-label="Dismiss notification"
        onClick={() => onDismiss(id)}
        className="shrink-0 opacity-60 hover:opacity-100 transition-opacity focus-visible:outline-none focus-visible:opacity-100"
      >
        <DismissIcon />
      </button>
    </div>
  );
}

export function ToastContainer({ toasts, position = "bottom-right" }: ToastContainerProps) {
  if (typeof document === "undefined" || toasts.length === 0) return null;

  return createPortal(
    <div
      aria-label="Notifications"
      className={cn(
        "fixed z-notification flex flex-col gap-3 pointer-events-none",
        positionClasses[position]
      )}
    >
      {toasts.map((toast) => (
        <Toast key={toast.id} {...toast} />
      ))}
    </div>,
    document.body
  );
}
