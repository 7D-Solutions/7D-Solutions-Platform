'use client';
// ============================================================
// NumericFormInput — number input with formatting
// ============================================================
import { forwardRef } from 'react';
import { clsx } from 'clsx';

export interface NumericFormInputProps
  extends Omit<React.InputHTMLAttributes<HTMLInputElement>, 'type'> {
  label: string;
  error?: string;
  hint?: string;
  prefix?: string;
  suffix?: string;
}

export const NumericFormInput = forwardRef<HTMLInputElement, NumericFormInputProps>(
  ({ label, error, hint, prefix, suffix, className, id, required, ...props }, ref) => {
    const inputId = id ?? label.toLowerCase().replace(/\s+/g, '-');

    return (
      <div className="flex flex-col gap-1">
        <label
          htmlFor={inputId}
          className="text-sm font-medium text-[--color-text-primary]"
        >
          {label}
          {required && <span className="ml-0.5 text-[--color-danger]">*</span>}
        </label>

        <div className="relative flex">
          {prefix && (
            <span className="flex items-center rounded-l-[--radius-default] border border-r-0 border-[--color-border-default] bg-[--color-bg-secondary] px-3 text-sm text-[--color-text-secondary]">
              {prefix}
            </span>
          )}
          <input
            ref={ref}
            id={inputId}
            type="number"
            required={required}
            aria-invalid={!!error}
            className={clsx(
              'flex-1 border px-3 py-2 text-sm',
              'text-[--color-text-primary] bg-[--color-bg-primary]',
              'focus:outline-none focus:ring-2 focus:ring-[--color-primary] focus:border-[--color-primary]',
              'disabled:bg-[--color-bg-secondary] disabled:cursor-not-allowed',
              prefix ? 'rounded-r-[--radius-default]' : 'rounded-[--radius-default]',
              suffix ? 'rounded-r-none' : '',
              error ? 'border-[--color-danger]' : 'border-[--color-border-default]',
              className
            )}
            {...props}
          />
          {suffix && (
            <span className="flex items-center rounded-r-[--radius-default] border border-l-0 border-[--color-border-default] bg-[--color-bg-secondary] px-3 text-sm text-[--color-text-secondary]">
              {suffix}
            </span>
          )}
        </div>

        {hint && !error && (
          <p className="text-xs text-[--color-text-secondary]">{hint}</p>
        )}
        {error && (
          <p role="alert" className="text-xs text-[--color-danger]">{error}</p>
        )}
      </div>
    );
  }
);

NumericFormInput.displayName = 'NumericFormInput';
