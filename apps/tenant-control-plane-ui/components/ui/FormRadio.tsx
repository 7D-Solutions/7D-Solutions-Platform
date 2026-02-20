'use client';
// ============================================================
// FormRadio — radio button group with label
// ============================================================
import { forwardRef } from 'react';
import { clsx } from 'clsx';

export interface RadioOption {
  value: string;
  label: string;
  hint?: string;
  disabled?: boolean;
}

export interface FormRadioProps {
  name: string;
  label: string;
  options: RadioOption[];
  value?: string;
  onChange?: (value: string) => void;
  error?: string;
  orientation?: 'horizontal' | 'vertical';
  required?: boolean;
}

export const FormRadio = forwardRef<HTMLFieldSetElement, FormRadioProps>(
  ({ name, label, options, value, onChange, error, orientation = 'vertical', required }, ref) => {
    return (
      <fieldset ref={ref} className="flex flex-col gap-1">
        <legend className="text-sm font-medium text-[--color-text-primary] mb-1">
          {label}
          {required && <span className="ml-0.5 text-[--color-danger]">*</span>}
        </legend>

        <div
          className={clsx(
            'flex gap-3',
            orientation === 'vertical' ? 'flex-col' : 'flex-row flex-wrap'
          )}
        >
          {options.map((opt) => (
            <label
              key={opt.value}
              className={clsx(
                'flex items-start gap-2 cursor-pointer',
                opt.disabled && 'opacity-50 cursor-not-allowed'
              )}
            >
              <input
                type="radio"
                name={name}
                value={opt.value}
                checked={value === opt.value}
                onChange={() => onChange?.(opt.value)}
                disabled={opt.disabled}
                className="mt-0.5 h-4 w-4 accent-[--color-primary] cursor-pointer disabled:cursor-not-allowed"
              />
              <span className="flex flex-col">
                <span className="text-sm text-[--color-text-primary]">{opt.label}</span>
                {opt.hint && (
                  <span className="text-xs text-[--color-text-secondary]">{opt.hint}</span>
                )}
              </span>
            </label>
          ))}
        </div>

        {error && (
          <p role="alert" className="text-xs text-[--color-danger]">{error}</p>
        )}
      </fieldset>
    );
  }
);

FormRadio.displayName = 'FormRadio';
