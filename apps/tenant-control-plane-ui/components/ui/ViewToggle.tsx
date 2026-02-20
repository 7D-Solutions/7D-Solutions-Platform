'use client';
// ============================================================
// ViewToggle — row/card toggle button group (presentational)
// Persistence is handled by usePersistedView hook, not this component.
// ============================================================
import { clsx } from 'clsx';
import { LayoutList, LayoutGrid } from 'lucide-react';

export type ViewMode = 'row' | 'card';

export interface ViewToggleProps {
  value: ViewMode;
  onChange: (mode: ViewMode) => void;
  className?: string;
}

export function ViewToggle({ value, onChange, className }: ViewToggleProps) {
  return (
    <div
      className={clsx(
        'inline-flex rounded-[--radius-default] border border-[--color-border-default] overflow-hidden',
        className
      )}
      role="group"
      aria-label="View mode"
    >
      <button
        onClick={() => onChange('row')}
        aria-label="Row view"
        aria-pressed={value === 'row'}
        className={clsx(
          'flex items-center gap-1.5 px-3 py-1.5 text-sm transition-[--transition-fast]',
          value === 'row'
            ? 'bg-[--color-primary] text-[--color-text-inverse]'
            : 'bg-[--color-bg-primary] text-[--color-text-secondary] hover:bg-[--color-bg-secondary]'
        )}
      >
        <LayoutList className="h-4 w-4" />
        <span>List</span>
      </button>
      <button
        onClick={() => onChange('card')}
        aria-label="Card view"
        aria-pressed={value === 'card'}
        className={clsx(
          'flex items-center gap-1.5 px-3 py-1.5 text-sm transition-[--transition-fast]',
          value === 'card'
            ? 'bg-[--color-primary] text-[--color-text-inverse]'
            : 'bg-[--color-bg-primary] text-[--color-text-secondary] hover:bg-[--color-bg-secondary]'
        )}
      >
        <LayoutGrid className="h-4 w-4" />
        <span>Cards</span>
      </button>
    </div>
  );
}
