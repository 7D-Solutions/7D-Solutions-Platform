'use client';
// ============================================================
// SearchableSelect — dropdown with search filter
// ============================================================
import { useState, useRef, useEffect } from 'react';
import { clsx } from 'clsx';
import { ChevronDown, Search, X } from 'lucide-react';

export interface SearchableSelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface SearchableSelectProps {
  label: string;
  options: SearchableSelectOption[];
  value?: string;
  onChange?: (value: string) => void;
  placeholder?: string;
  error?: string;
  hint?: string;
  disabled?: boolean;
  required?: boolean;
  clearable?: boolean;
}

export function SearchableSelect({
  label,
  options,
  value,
  onChange,
  placeholder = 'Select...',
  error,
  hint,
  disabled,
  required,
  clearable = false,
}: SearchableSelectProps) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState('');
  const ref = useRef<HTMLDivElement>(null);
  const inputId = label.toLowerCase().replace(/\s+/g, '-');

  const filtered = options.filter((o) =>
    o.label.toLowerCase().includes(search.toLowerCase())
  );

  const selected = options.find((o) => o.value === value);

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
        setSearch('');
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  return (
    <div className="flex flex-col gap-1">
      <label
        htmlFor={inputId}
        className="text-sm font-medium text-[--color-text-primary]"
      >
        {label}
        {required && <span className="ml-0.5 text-[--color-danger]">*</span>}
      </label>

      <div ref={ref} className="relative">
        <button
          id={inputId}
          type="button"
          disabled={disabled}
          onClick={() => !disabled && setOpen(!open)}
          className={clsx(
            'flex w-full items-center justify-between rounded-[--radius-default] border px-3 py-2 text-sm',
            'bg-[--color-bg-primary] text-left transition-[--transition-fast]',
            'focus:outline-none focus:ring-2 focus:ring-[--color-primary]',
            'disabled:bg-[--color-bg-secondary] disabled:cursor-not-allowed',
            error ? 'border-[--color-danger]' : 'border-[--color-border-default]',
            open && 'ring-2 ring-[--color-primary] border-[--color-primary]'
          )}
        >
          <span className={clsx(!selected && 'text-[--color-text-muted]')}>
            {selected?.label ?? placeholder}
          </span>
          <div className="flex items-center gap-1">
            {clearable && selected && (
              <span
                role="button"
                onClick={(e) => { e.stopPropagation(); onChange?.(''); }}
                className="rounded p-0.5 hover:bg-[--color-bg-tertiary]"
              >
                <X className="h-3 w-3 text-[--color-text-secondary]" />
              </span>
            )}
            <ChevronDown className={clsx('h-4 w-4 text-[--color-text-secondary] transition-[--transition-fast]', open && 'rotate-180')} />
          </div>
        </button>

        {open && (
          <div
            className="absolute z-[--z-dropdown] mt-1 w-full rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] shadow-[--shadow-lg]"
          >
            <div className="border-b border-[--color-border-light] p-2">
              <div className="flex items-center gap-2 rounded-[--radius-sm] border border-[--color-border-default] px-2 py-1">
                <Search className="h-3.5 w-3.5 text-[--color-text-muted]" />
                <input
                  autoFocus
                  type="text"
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  placeholder="Search..."
                  className="flex-1 text-sm outline-none bg-transparent text-[--color-text-primary] placeholder:text-[--color-text-muted]"
                />
              </div>
            </div>
            <ul className="max-h-48 overflow-y-auto py-1">
              {filtered.length === 0 ? (
                <li className="px-3 py-2 text-sm text-[--color-text-muted]">No results</li>
              ) : (
                filtered.map((opt) => (
                  <li
                    key={opt.value}
                    onClick={() => {
                      if (!opt.disabled) {
                        onChange?.(opt.value);
                        setOpen(false);
                        setSearch('');
                      }
                    }}
                    className={clsx(
                      'px-3 py-2 text-sm cursor-pointer transition-[--transition-fast]',
                      opt.disabled
                        ? 'opacity-50 cursor-not-allowed'
                        : 'hover:bg-[--color-bg-secondary]',
                      opt.value === value && 'bg-[--color-bg-secondary] font-medium'
                    )}
                  >
                    {opt.label}
                  </li>
                ))
              )}
            </ul>
          </div>
        )}
      </div>

      {hint && !error && <p className="text-xs text-[--color-text-secondary]">{hint}</p>}
      {error && <p role="alert" className="text-xs text-[--color-danger]">{error}</p>}
    </div>
  );
}
