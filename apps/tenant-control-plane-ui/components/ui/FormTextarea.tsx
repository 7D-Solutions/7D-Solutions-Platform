'use client';
// ============================================================
// FormTextarea — multi-line text with label and character count
// ============================================================
import { forwardRef } from 'react';
import { clsx } from 'clsx';

export interface FormTextareaProps extends React.TextareaHTMLAttributes<HTMLTextAreaElement> {
  label: string;
  error?: string;
  hint?: string;
  showCharCount?: boolean;
}

export const FormTextarea = forwardRef<HTMLTextAreaElement, FormTextareaProps>(
  ({ label, error, hint, showCharCount, className, id, required, maxLength, value, ...props }, ref) => {
    const textareaId = id ?? label.toLowerCase().replace(/\s+/g, '-');
    const currentLength = typeof value === 'string' ? value.length : 0;

    return (
      <div className="flex flex-col gap-1">
        <label
          htmlFor={textareaId}
          className="text-sm font-medium text-[--color-text-primary]"
        >
          {label}
          {required && <span className="ml-0.5 text-[--color-danger]">*</span>}
        </label>

        <textarea
          ref={ref}
          id={textareaId}
          required={required}
          maxLength={maxLength}
          value={value}
          aria-invalid={!!error}
          className={clsx(
            'rounded-[--radius-default] border px-3 py-2 text-sm min-h-[80px] resize-y',
            'text-[--color-text-primary] placeholder:text-[--color-text-muted]',
            'bg-[--color-bg-primary] [transition:var(--transition-fast)]',
            'focus:outline-none focus:ring-2 focus:ring-[--color-primary] focus:border-[--color-primary]',
            'disabled:bg-[--color-bg-secondary] disabled:cursor-not-allowed',
            error ? 'border-[--color-danger]' : 'border-[--color-border-default]',
            className
          )}
          {...props}
        />

        <div className="flex items-center justify-between">
          <div>
            {hint && !error && (
              <p className="text-xs text-[--color-text-secondary]">{hint}</p>
            )}
            {error && (
              <p role="alert" className="text-xs text-[--color-danger]">{error}</p>
            )}
          </div>
          {showCharCount && maxLength && (
            <p className="text-xs text-[--color-text-muted]">
              {currentLength}/{maxLength}
            </p>
          )}
        </div>
      </div>
    );
  }
);

FormTextarea.displayName = 'FormTextarea';
