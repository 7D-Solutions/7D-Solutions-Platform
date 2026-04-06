import React, { useCallback, useRef, useState } from "react";
import { cn } from "../lib/cn";
import { Spinner } from "./Spinner";

export type ButtonVariant = "primary" | "secondary" | "danger" | "ghost" | "outline";
export type ButtonSize = "xs" | "sm" | "md" | "lg";

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
  doubleClickProtection?: boolean;
  leftIcon?: React.ReactNode;
  rightIcon?: React.ReactNode;
}

const variantClasses: Record<ButtonVariant, string> = {
  primary: "bg-primary text-text-inverse hover:bg-primary-dark focus-visible:ring-primary disabled:bg-primary/50",
  secondary: "bg-secondary text-text-inverse hover:bg-secondary-dark focus-visible:ring-secondary disabled:bg-secondary/50",
  danger: "bg-danger text-text-inverse hover:bg-danger-dark focus-visible:ring-danger disabled:bg-danger/50",
  ghost: "bg-transparent text-text-primary hover:bg-gray-100 focus-visible:ring-gray-400 disabled:text-text-muted",
  outline: "bg-transparent border border-border text-text-primary hover:bg-gray-100 focus-visible:ring-gray-400 disabled:text-text-muted disabled:border-border-light",
};

const sizeClasses: Record<ButtonSize, string> = {
  xs: "h-7 px-3 text-xs gap-1",
  sm: "h-8 px-4 text-sm gap-1.5",
  md: "h-9 px-5 text-base gap-2",
  lg: "h-11 px-6 text-lg gap-2",
};

const spinnerSizes: Record<ButtonSize, "xs" | "sm" | "md"> = {
  xs: "xs",
  sm: "xs",
  md: "sm",
  lg: "sm",
};

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  function Button(
    {
      variant = "primary",
      size = "md",
      loading = false,
      doubleClickProtection = false,
      leftIcon,
      rightIcon,
      disabled,
      onClick,
      children,
      className,
      type = "button",
      ...rest
    },
    ref
  ) {
    const [blocked, setBlocked] = useState(false);
    const blockTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    const handleClick = useCallback(
      (e: React.MouseEvent<HTMLButtonElement>) => {
        if (blocked || loading) return;
        if (doubleClickProtection) {
          setBlocked(true);
          blockTimerRef.current = setTimeout(() => setBlocked(false), 300);
        }
        onClick?.(e);
      },
      [blocked, loading, doubleClickProtection, onClick]
    );

    const isDisabled = disabled || loading || blocked;

    return (
      <button
        ref={ref}
        type={type}
        disabled={isDisabled}
        aria-disabled={isDisabled}
        aria-busy={loading}
        onClick={handleClick}
        className={cn(
          "inline-flex items-center justify-center font-medium rounded-md",
          "transition-all duration-150",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-offset-2",
          "disabled:cursor-not-allowed disabled:opacity-60",
          variantClasses[variant],
          sizeClasses[size],
          className
        )}
        {...rest}
      >
        {loading ? (
          <Spinner size={spinnerSizes[size]} className="shrink-0" />
        ) : (
          leftIcon && <span className="shrink-0">{leftIcon}</span>
        )}
        {children}
        {!loading && rightIcon && <span className="shrink-0">{rightIcon}</span>}
      </button>
    );
  }
);
