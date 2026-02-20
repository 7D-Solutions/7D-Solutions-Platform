'use client';
// ============================================================
// FormCheckbox — checkbox with label and error state (for forms)
// For table/grid checkboxes without labels, use Checkbox instead.
// ============================================================
import { forwardRef } from 'react';
import { clsx } from 'clsx';

export interface FormCheckboxProps extends React.InputHTMLAttributes<HTMLInputElement> {
  label: string;
  error?: string;
  hint?: string;
}

export const FormCheckbox = forwardRef<HTMLInputElement, FormCheckboxProps>(
  ({ label, error, hint, className, id, ...props }, ref) => {
    const inputId = id ?? label.toLowerCase().replace(/\s+/g, '-');

    return (
      <div className="flex flex-col gap-1">
        <label
          htmlFor={inputId}
          className="flex items-start gap-2 cursor-pointer"
        >
          <input
            ref={ref}
            id={inputId}
            type="checkbox"
            aria-invalid={!!error}
            className={clsx(
              'mt-0.5 h-4 w-4 rounded-[--radius-sm] border-[--color-border-default]',
              'accent-[--color-primary] cursor-pointer',
              'disabled:cursor-not-allowed disabled:opacity-50',
              className
            )}
            {...props}
          />
          <span className="text-sm text-[--color-text-primary]">{label}</span>
        </label>

        {hint && !error && (
          <p className="ml-6 text-xs text-[--color-text-secondary]">{hint}</p>
        )}
        {error && (
          <p role="alert" className="ml-6 text-xs text-[--color-danger]">{error}</p>
        )}
      </div>
    );
  }
);

FormCheckbox.displayName = 'FormCheckbox';
