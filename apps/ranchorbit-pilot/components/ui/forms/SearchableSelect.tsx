import React, { useCallback, useEffect, useId, useRef, useState } from "react";
import { cn } from "../lib/cn";
import { ariaDescribedBy, ariaInvalid } from "../lib/a11y";
import { Keys } from "../lib/keyboard";

export interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export type SearchableSelectSize = "sm" | "md" | "lg";

export interface SearchableSelectProps {
  options: SelectOption[];
  value?: string;
  onChange?: (value: string) => void;
  placeholder?: string;
  searchPlaceholder?: string;
  disabled?: boolean;
  error?: boolean;
  describedBy?: string;
  size?: SearchableSelectSize;
  clearable?: boolean;
  emptyMessage?: string;
  className?: string;
  "aria-label"?: string;
  id?: string;
}

const sizeClasses: Record<SearchableSelectSize, string> = {
  sm: "h-8 px-3 text-sm",
  md: "h-9 px-3 text-base",
  lg: "h-11 px-4 text-lg",
};

const ChevronDown = () => (
  <svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <polyline points="4 6 8 10 12 6" />
  </svg>
);

const ClearIcon = () => (
  <svg aria-hidden="true" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <line x1="3" y1="3" x2="13" y2="13" /><line x1="13" y1="3" x2="3" y2="13" />
  </svg>
);

export const SearchableSelect = React.forwardRef<HTMLButtonElement, SearchableSelectProps>(
  function SearchableSelect(
    {
      options,
      value,
      onChange,
      placeholder = "Select…",
      searchPlaceholder = "Search…",
      disabled = false,
      error = false,
      describedBy,
      size = "md",
      clearable = false,
      emptyMessage = "No options found",
      className,
      "aria-label": ariaLabel,
      id: externalId,
    },
    ref
  ) {
    const internalId = useId();
    const id = externalId ?? internalId;
    const listboxId = `${id}-listbox`;
    const searchId = `${id}-search`;

    const [open, setOpen] = useState(false);
    const [query, setQuery] = useState("");
    const [activeIndex, setActiveIndex] = useState<number>(-1);

    const containerRef = useRef<HTMLDivElement>(null);
    const searchRef = useRef<HTMLInputElement>(null);
    const listRef = useRef<HTMLUListElement>(null);

    const selected = options.find((o) => o.value === value);
    const filtered = query
      ? options.filter(
          (o) =>
            o.label.toLowerCase().includes(query.toLowerCase()) ||
            o.value.toLowerCase().includes(query.toLowerCase())
        )
      : options;

    const openDropdown = useCallback(() => {
      if (disabled) return;
      setOpen(true);
      setQuery("");
      setActiveIndex(selected ? options.indexOf(selected) : 0);
    }, [disabled, selected, options]);

    const closeDropdown = useCallback(() => {
      setOpen(false);
      setQuery("");
      setActiveIndex(-1);
    }, []);

    const selectOption = useCallback(
      (option: SelectOption) => {
        if (option.disabled) return;
        onChange?.(option.value);
        closeDropdown();
      },
      [onChange, closeDropdown]
    );

    const clearSelection = useCallback(
      (e: React.MouseEvent) => {
        e.stopPropagation();
        onChange?.("");
      },
      [onChange]
    );

    useEffect(() => {
      if (!open) return;
      const handle = (e: MouseEvent) => {
        if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
          closeDropdown();
        }
      };
      document.addEventListener("mousedown", handle);
      return () => document.removeEventListener("mousedown", handle);
    }, [open, closeDropdown]);

    useEffect(() => {
      if (open) requestAnimationFrame(() => searchRef.current?.focus());
    }, [open]);

    useEffect(() => {
      if (activeIndex < 0 || !listRef.current) return;
      const items = listRef.current.querySelectorAll<HTMLElement>("[role=option]");
      items[activeIndex]?.scrollIntoView({ block: "nearest" });
    }, [activeIndex]);

    const handleTriggerKeyDown = (e: React.KeyboardEvent) => {
      if (e.key === Keys.Enter || e.key === Keys.Space || e.key === Keys.ArrowDown) {
        e.preventDefault();
        openDropdown();
      }
    };

    const handleSearchKeyDown = (e: React.KeyboardEvent) => {
      if (e.key === Keys.Escape) { closeDropdown(); return; }
      if (e.key === Keys.ArrowDown) { e.preventDefault(); setActiveIndex((i) => Math.min(i + 1, filtered.length - 1)); return; }
      if (e.key === Keys.ArrowUp) { e.preventDefault(); setActiveIndex((i) => Math.max(i - 1, 0)); return; }
      if (e.key === Keys.Enter) {
        e.preventDefault();
        const opt = filtered[activeIndex];
        if (opt && !opt.disabled) selectOption(opt);
        return;
      }
      if (e.key === Keys.Tab) closeDropdown();
    };

    const showClear = clearable && value && !disabled;

    return (
      <div ref={containerRef} className={cn("relative", className)}>
        <button
          ref={ref}
          type="button"
          id={id}
          role="combobox"
          aria-haspopup="listbox"
          aria-expanded={open}
          aria-controls={listboxId}
          aria-label={ariaLabel}
          aria-invalid={ariaInvalid(error)}
          aria-describedby={ariaDescribedBy(describedBy)}
          disabled={disabled}
          onClick={() => (open ? closeDropdown() : openDropdown())}
          onKeyDown={handleTriggerKeyDown}
          className={cn(
            "inline-flex items-center justify-between w-full rounded-md border bg-bg-primary text-text-primary",
            "transition-colors duration-150",
            "focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary",
            "disabled:cursor-not-allowed disabled:bg-bg-secondary disabled:text-text-muted",
            error ? "border-danger focus:ring-danger focus:border-danger" : "border-border hover:border-border-dark",
            sizeClasses[size]
          )}
        >
          <span className={cn("truncate", !selected && "text-text-muted")}>
            {selected ? selected.label : placeholder}
          </span>
          <span className="flex items-center gap-1 ml-2 shrink-0">
            {showClear && (
              <span
                role="button"
                tabIndex={-1}
                aria-label="Clear selection"
                onClick={clearSelection}
                className="text-text-muted hover:text-text-primary p-0.5 rounded"
              >
                <ClearIcon />
              </span>
            )}
            <span className={cn("text-text-muted", open && "rotate-180 transition-transform")}>
              <ChevronDown />
            </span>
          </span>
        </button>

        {open && (
          <div className={cn("absolute z-50 top-full left-0 right-0 mt-1", "bg-bg-primary border border-border rounded-md shadow-lg", "flex flex-col overflow-hidden")}>
            <div className="p-2 border-b border-border">
              <input
                ref={searchRef}
                id={searchId}
                type="text"
                role="searchbox"
                aria-label="Filter options"
                aria-controls={listboxId}
                value={query}
                onChange={(e) => { setQuery(e.target.value); setActiveIndex(0); }}
                onKeyDown={handleSearchKeyDown}
                placeholder={searchPlaceholder}
                className={cn("block w-full rounded border border-border px-2 py-1 text-sm bg-bg-primary text-text-primary", "placeholder:text-text-muted", "focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary")}
              />
            </div>
            <ul ref={listRef} id={listboxId} role="listbox" aria-label="Options" className="overflow-y-auto max-h-60 py-1">
              {filtered.length === 0 ? (
                <li className="px-3 py-2 text-sm text-text-muted text-center select-none" aria-live="polite">{emptyMessage}</li>
              ) : (
                filtered.map((option, index) => (
                  <li
                    key={option.value}
                    id={`${listboxId}-${option.value}`}
                    role="option"
                    aria-selected={option.value === value}
                    aria-disabled={option.disabled}
                    onClick={() => selectOption(option)}
                    onMouseEnter={() => setActiveIndex(index)}
                    className={cn(
                      "px-3 py-2 text-sm cursor-pointer select-none transition-colors duration-100",
                      option.disabled ? "text-text-muted cursor-not-allowed" : "text-text-primary hover:bg-gray-100",
                      option.value === value && "font-medium text-primary",
                      index === activeIndex && !option.disabled && "bg-gray-100"
                    )}
                  >
                    {option.label}
                  </li>
                ))
              )}
            </ul>
          </div>
        )}
      </div>
    );
  }
);
