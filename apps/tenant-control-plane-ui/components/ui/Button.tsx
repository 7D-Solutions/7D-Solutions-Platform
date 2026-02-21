'use client';
// ============================================================
// Button — all variants, all sizes, double-click protection ON by default
// Rule: Never use raw <button> elements. Always import this.
// ============================================================
import { forwardRef, useRef, useState } from 'react';
import { clsx } from 'clsx';
import { Loader2 } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { BUTTON_COOLDOWN_MS } from '@/lib/constants';

export type ButtonVariant =
  | 'primary' | 'secondary' | 'success' | 'danger' | 'warning' | 'info' | 'ghost' | 'outline';

export type ButtonSize = 'compact' | 'xs' | 'sm' | 'md' | 'lg' | 'xl';

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
  active?: boolean;
  icon?: LucideIcon;
  iconPosition?: 'left' | 'right';
  /** Disable double-click protection — requires a comment justification */
  disableCooldown?: boolean;
}

const variantClasses: Record<ButtonVariant, string> = {
  primary:   'bg-[--color-primary] text-[--color-text-inverse] hover:bg-[--color-primary-dark] border-transparent',
  secondary: 'bg-[--color-secondary] text-[--color-text-inverse] hover:bg-[--color-secondary-dark] border-transparent',
  success:   'bg-[--color-success] text-[--color-text-inverse] hover:bg-[--color-success-dark] border-transparent',
  danger:    'bg-[--color-danger] text-[--color-text-inverse] hover:bg-[--color-danger-dark] border-transparent',
  warning:   'bg-[--color-warning] text-[--color-text-primary] hover:bg-[--color-warning-dark] border-transparent',
  info:      'bg-[--color-info] text-[--color-text-inverse] hover:bg-[--color-info-dark] border-transparent',
  ghost:     'bg-transparent text-[--color-text-secondary] hover:bg-[--color-bg-tertiary] border-transparent',
  outline:   'bg-transparent text-[--color-primary] hover:bg-[--color-bg-secondary] border-[--color-primary]',
};

const sizeClasses: Record<ButtonSize, string> = {
  compact: 'px-[--component-size-compact-padding-x] py-[--component-size-compact-padding-y] text-[length:var(--component-size-compact-font-size)] min-h-[--component-size-compact-min-height] leading-[--component-size-compact-line-height]',
  xs:      'px-[--component-size-xs-padding-x] py-[--component-size-xs-padding-y] text-[length:var(--component-size-xs-font-size)] min-h-[--component-size-xs-min-height]',
  sm:      'px-[--component-size-sm-padding-x] py-[--component-size-sm-padding-y] text-[length:var(--component-size-sm-font-size)] min-h-[--component-size-sm-min-height]',
  md:      'px-[--component-size-md-padding-x] py-[--component-size-md-padding-y] text-[length:var(--component-size-md-font-size)] min-h-[--component-size-md-min-height]',
  lg:      'px-[--component-size-lg-padding-x] py-[--component-size-lg-padding-y] text-[length:var(--component-size-lg-font-size)] min-h-[--component-size-lg-min-height]',
  xl:      'px-[--component-size-xl-padding-x] py-[--component-size-xl-padding-y] text-[length:var(--component-size-xl-font-size)] min-h-[--component-size-xl-min-height]',
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  (
    {
      variant = 'primary',
      size = 'md',
      loading = false,
      active = false,
      icon: Icon,
      iconPosition = 'left',
      disableCooldown = false,
      disabled,
      onClick,
      children,
      className,
      ...props
    },
    ref
  ) => {
    const [onCooldown, setOnCooldown] = useState(false);
    const cooldownRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    const handleClick = (e: React.MouseEvent<HTMLButtonElement>) => {
      if (onCooldown || loading || disabled) return;

      if (!disableCooldown) {
        setOnCooldown(true);
        cooldownRef.current = setTimeout(() => setOnCooldown(false), BUTTON_COOLDOWN_MS);
      }

      onClick?.(e);
    };

    const isDisabled = disabled || loading || onCooldown;

    return (
      <button
        ref={ref}
        disabled={isDisabled}
        onClick={handleClick}
        className={clsx(
          'inline-flex items-center justify-center gap-2 rounded-[--radius-default] border font-medium',
          '[transition:var(--transition-default)] focus-visible:outline-none focus-visible:ring-2',
          'focus-visible:ring-[--color-primary] focus-visible:ring-offset-2',
          'disabled:opacity-50 disabled:cursor-not-allowed',
          variantClasses[variant],
          sizeClasses[size],
          active && 'opacity-100 ring-2 ring-[--color-primary]',
          className
        )}
        {...props}
      >
        {loading && <Loader2 className="h-4 w-4 animate-spin" />}
        {!loading && Icon && iconPosition === 'left' && <Icon className="h-4 w-4" />}
        {children}
        {!loading && Icon && iconPosition === 'right' && <Icon className="h-4 w-4" />}
      </button>
    );
  }
);

Button.displayName = 'Button';
