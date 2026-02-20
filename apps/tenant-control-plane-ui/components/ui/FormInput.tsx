'use client';
// ============================================================
// FormInput — text input with label, error, required indicator
// Rule: Never use raw <input>. Always import this.
// ============================================================
import { forwardRef } from 'react';
import { clsx } from 'clsx';

export interface FormInputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  label: string;
  error?: string;
  hint?: string;
  hideLabel?: boolean;
}

export const FormInput = forwardRef<HTMLInputElement, FormInputProps>(
  ({ label, error, hint, hideLabel, className, id, required, ...props }, ref) => {
    const inputId = id ?? label.toLowerCase().replace(/\s+/g, '-');

    return (
      <div className="flex flex-col gap-1">
        <label
          htmlFor={inputId}
          className={clsx(
            'text-sm font-medium text-[--color-text-primary]',
            hideLabel && 'sr-only'
          )}
        >
          {label}
          {required && <span className="ml-0.5 text-[--color-danger]">*</span>}
        </label>

        <input
          ref={ref}
          id={inputId}
          required={required}
          aria-invalid={!!error}
          aria-describedby={error ? `${inputId}-error` : hint ? `${inputId}-hint` : undefined}
          className={clsx(
            'rounded-[--radius-default] border px-3 py-2 text-sm',
            'text-[--color-text-primary] placeholder:text-[--color-text-muted]',
            'bg-[--color-bg-primary] transition-[--transition-fast]',
            'focus:outline-none focus:ring-2 focus:ring-[--color-primary] focus:border-[--color-primary]',
            'disabled:bg-[--color-bg-secondary] disabled:cursor-not-allowed',
            error
              ? 'border-[--color-danger] focus:ring-[--color-danger]'
              : 'border-[--color-border-default]',
            className
          )}
          {...props}
        />

        {hint && !error && (
          <p id={`${inputId}-hint`} className="text-xs text-[--color-text-secondary]">
            {hint}
          </p>
        )}
        {error && (
          <p id={`${inputId}-error`} role="alert" className="text-xs text-[--color-danger]">
            {error}
          </p>
        )}
      </div>
    );
  }
);

FormInput.displayName = 'FormInput';
