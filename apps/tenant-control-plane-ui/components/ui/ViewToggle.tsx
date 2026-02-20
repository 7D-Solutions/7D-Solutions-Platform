'use client';
// ============================================================
// ViewToggle — row/card toggle, preference persisted per table per user
// ============================================================
import { useEffect } from 'react';
import { clsx } from 'clsx';
import { LayoutList, LayoutGrid } from 'lucide-react';
import { userPreferencesService } from '@/infrastructure/services/userPreferencesService';

export type ViewMode = 'row' | 'card';

interface ViewToggleProps {
  tableId: string;
  value: ViewMode;
  onChange: (mode: ViewMode) => void;
  className?: string;
}

export function ViewToggle({ tableId, value, onChange, className }: ViewToggleProps) {
  const prefKey = `view-mode-${tableId}`;

  // Load persisted preference on mount
  useEffect(() => {
    userPreferencesService.getPreference<ViewMode>(prefKey, null).then((saved) => {
      if (saved && saved !== value) onChange(saved);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [prefKey]);

  const handleChange = (mode: ViewMode) => {
    onChange(mode);
    userPreferencesService.savePreference(prefKey, mode);
  };

  return (
    <div
      className={clsx(
        'inline-flex rounded-[--radius-default] border border-[--color-border-default] overflow-hidden',
        className
      )}
    >
      <button
        onClick={() => handleChange('row')}
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
        onClick={() => handleChange('card')}
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
