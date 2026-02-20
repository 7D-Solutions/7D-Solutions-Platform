'use client';
// ============================================================
// FormSelect — dropdown select with label, error display
// ============================================================
import { forwardRef } from 'react';
import { clsx } from 'clsx';

export interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface FormSelectProps extends React.SelectHTMLAttributes<HTMLSelectElement> {
  label: string;
  options: SelectOption[];
  error?: string;
  hint?: string;
  placeholder?: string;
}

export const FormSelect = forwardRef<HTMLSelectElement, FormSelectProps>(
  ({ label, options, error, hint, placeholder, className, id, required, ...props }, ref) => {
    const selectId = id ?? label.toLowerCase().replace(/\s+/g, '-');

    return (
      <div className="flex flex-col gap-1">
        <label
          htmlFor={selectId}
          className="text-sm font-medium text-[--color-text-primary]"
        >
          {label}
          {required && <span className="ml-0.5 text-[--color-danger]">*</span>}
        </label>

        <select
          ref={ref}
          id={selectId}
          required={required}
          aria-invalid={!!error}
          className={clsx(
            'rounded-[--radius-default] border px-3 py-2 text-sm',
            'text-[--color-text-primary] bg-[--color-bg-primary]',
            'focus:outline-none focus:ring-2 focus:ring-[--color-primary] focus:border-[--color-primary]',
            'disabled:bg-[--color-bg-secondary] disabled:cursor-not-allowed',
            error ? 'border-[--color-danger]' : 'border-[--color-border-default]',
            className
          )}
          {...props}
        >
          {placeholder && (
            <option value="" disabled>
              {placeholder}
            </option>
          )}
          {options.map((opt) => (
            <option key={opt.value} value={opt.value} disabled={opt.disabled}>
              {opt.label}
            </option>
          ))}
        </select>

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

FormSelect.displayName = 'FormSelect';
