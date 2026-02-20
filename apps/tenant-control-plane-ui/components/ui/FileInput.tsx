'use client';
// ============================================================
// FileInput — file upload with drag-drop support
// ============================================================
import { useState, useRef } from 'react';
import { clsx } from 'clsx';
import { Upload, X, File } from 'lucide-react';

export interface FileInputProps {
  label: string;
  accept?: string;
  multiple?: boolean;
  error?: string;
  hint?: string;
  disabled?: boolean;
  required?: boolean;
  onChange?: (files: File[]) => void;
}

export function FileInput({
  label,
  accept,
  multiple,
  error,
  hint,
  disabled,
  required,
  onChange,
}: FileInputProps) {
  const [dragging, setDragging] = useState(false);
  const [files, setFiles] = useState<File[]>([]);
  const inputRef = useRef<HTMLInputElement>(null);
  const inputId = label.toLowerCase().replace(/\s+/g, '-');

  const handleFiles = (newFiles: FileList | null) => {
    if (!newFiles) return;
    const arr = Array.from(newFiles);
    const updated = multiple ? [...files, ...arr] : arr;
    setFiles(updated);
    onChange?.(updated);
  };

  const removeFile = (index: number) => {
    const updated = files.filter((_, i) => i !== index);
    setFiles(updated);
    onChange?.(updated);
  };

  return (
    <div className="flex flex-col gap-1">
      <label
        htmlFor={inputId}
        className="text-sm font-medium text-[--color-text-primary]"
      >
        {label}
        {required && <span className="ml-0.5 text-[--color-danger]">*</span>}
      </label>

      <div
        onClick={() => !disabled && inputRef.current?.click()}
        onDragOver={(e) => { e.preventDefault(); if (!disabled) setDragging(true); }}
        onDragLeave={() => setDragging(false)}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          if (!disabled) handleFiles(e.dataTransfer.files);
        }}
        className={clsx(
          'flex flex-col items-center justify-center gap-2 rounded-[--radius-lg] border-2 border-dashed px-6 py-8',
          'cursor-pointer transition-[--transition-fast]',
          disabled && 'opacity-50 cursor-not-allowed',
          dragging
            ? 'border-[--color-primary] bg-[--color-bg-secondary]'
            : error
            ? 'border-[--color-danger] hover:border-[--color-danger]'
            : 'border-[--color-border-default] hover:border-[--color-primary]'
        )}
      >
        <Upload className="h-8 w-8 text-[--color-text-muted]" />
        <div className="text-center">
          <p className="text-sm font-medium text-[--color-text-primary]">
            Drag and drop or click to upload
          </p>
          {accept && (
            <p className="text-xs text-[--color-text-secondary]">
              Accepted: {accept}
            </p>
          )}
        </div>
        <input
          ref={inputRef}
          id={inputId}
          type="file"
          accept={accept}
          multiple={multiple}
          disabled={disabled}
          className="sr-only"
          onChange={(e) => handleFiles(e.target.files)}
        />
      </div>

      {files.length > 0 && (
        <ul className="mt-2 flex flex-col gap-1">
          {files.map((file, i) => (
            <li
              key={`${file.name}-${i}`}
              className="flex items-center gap-2 rounded-[--radius-default] border border-[--color-border-light] bg-[--color-bg-secondary] px-3 py-2"
            >
              <File className="h-4 w-4 text-[--color-text-secondary]" />
              <span className="flex-1 text-sm text-[--color-text-primary] truncate">{file.name}</span>
              <button
                type="button"
                onClick={(e) => { e.stopPropagation(); removeFile(i); }}
                className="rounded p-0.5 hover:bg-[--color-bg-tertiary]"
                aria-label={`Remove ${file.name}`}
              >
                <X className="h-3.5 w-3.5 text-[--color-text-secondary]" />
              </button>
            </li>
          ))}
        </ul>
      )}

      {hint && !error && <p className="text-xs text-[--color-text-secondary]">{hint}</p>}
      {error && <p role="alert" className="text-xs text-[--color-danger]">{error}</p>}
    </div>
  );
}
