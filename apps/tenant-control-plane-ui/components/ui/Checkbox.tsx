'use client';
// ============================================================
// Checkbox — simple checkbox for tables/grids (no label wrapper)
// For form checkboxes with labels, use FormCheckbox instead.
// ============================================================
import { forwardRef } from 'react';
import { clsx } from 'clsx';

export interface CheckboxProps extends React.InputHTMLAttributes<HTMLInputElement> {
  indeterminate?: boolean;
}

export const Checkbox = forwardRef<HTMLInputElement, CheckboxProps>(
  ({ indeterminate, className, ...props }, ref) => {
    const setRef = (el: HTMLInputElement | null) => {
      if (el) {
        el.indeterminate = indeterminate ?? false;
        if (typeof ref === 'function') ref(el);
        else if (ref) ref.current = el;
      }
    };

    return (
      <input
        type="checkbox"
        ref={setRef}
        className={clsx(
          'h-4 w-4 rounded-[--radius-sm] border-[--color-border-default]',
          'accent-[--color-primary] cursor-pointer',
          'disabled:cursor-not-allowed disabled:opacity-50',
          className
        )}
        {...props}
      />
    );
  }
);

Checkbox.displayName = 'Checkbox';
