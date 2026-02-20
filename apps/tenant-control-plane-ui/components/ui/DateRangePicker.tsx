'use client';
// ============================================================
// DateRangePicker — date range input (start + end dates)
// Used in audit log filters, billing date ranges, etc.
// ============================================================
import { clsx } from 'clsx';

export interface DateRange {
  start: string;
  end: string;
}

export interface DateRangePickerProps {
  label?: string;
  value: DateRange;
  onChange: (range: DateRange) => void;
  error?: string;
  hint?: string;
  disabled?: boolean;
  required?: boolean;
}

export function DateRangePicker({
  label,
  value,
  onChange,
  error,
  hint,
  disabled,
  required,
}: DateRangePickerProps) {
  const inputBase = clsx(
    'flex-1 rounded-[--radius-default] border px-3 py-2 text-sm',
    'text-[--color-text-primary] bg-[--color-bg-primary]',
    'focus:outline-none focus:ring-2 focus:ring-[--color-primary] focus:border-[--color-primary]',
    'disabled:bg-[--color-bg-secondary] disabled:cursor-not-allowed',
    error ? 'border-[--color-danger]' : 'border-[--color-border-default]'
  );

  return (
    <div className="flex flex-col gap-1">
      {label && (
        <span className="text-sm font-medium text-[--color-text-primary]">
          {label}
          {required && <span className="ml-0.5 text-[--color-danger]">*</span>}
        </span>
      )}

      <div className="flex items-center gap-2">
        <input
          type="date"
          value={value.start}
          onChange={(e) => onChange({ ...value, start: e.target.value })}
          disabled={disabled}
          aria-label="Start date"
          className={inputBase}
        />
        <span className="text-sm text-[--color-text-secondary]">to</span>
        <input
          type="date"
          value={value.end}
          min={value.start}
          onChange={(e) => onChange({ ...value, end: e.target.value })}
          disabled={disabled}
          aria-label="End date"
          className={inputBase}
        />
      </div>

      {hint && !error && <p className="text-xs text-[--color-text-secondary]">{hint}</p>}
      {error && <p role="alert" className="text-xs text-[--color-danger]">{error}</p>}
    </div>
  );
}
