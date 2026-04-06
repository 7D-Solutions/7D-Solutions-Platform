import React, { useCallback, useId, useRef, useState } from "react";
import { cn } from "../../lib/cn.js";

export interface FileUploadProps {
  /** Accepted MIME types or extensions, e.g. "image/*,.pdf" */
  accept?: string;
  multiple?: boolean;
  /** Maximum allowed file size in bytes */
  maxSizeBytes?: number;
  onFilesChange: (files: File[]) => void;
  /** Controlled — current files list */
  files?: File[];
  disabled?: boolean;
  /** Validation error message */
  error?: string;
  hint?: string;
  /** Screen-reader / visible label for the drop zone */
  label?: string;
  className?: string;
  id?: string;
}

const UploadIcon = () => (
  <svg
    aria-hidden="true"
    width="24"
    height="24"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.5"
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
    <polyline points="17 8 12 3 7 8" />
    <line x1="12" y1="3" x2="12" y2="15" />
  </svg>
);

const RemoveIcon = () => (
  <svg
    aria-hidden="true"
    width="14"
    height="14"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
  >
    <line x1="3" y1="3" x2="13" y2="13" />
    <line x1="13" y1="3" x2="3" y2="13" />
  </svg>
);

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function FileUpload({
  accept,
  multiple = false,
  maxSizeBytes,
  onFilesChange,
  files = [],
  disabled = false,
  error,
  hint,
  label = "Upload files",
  className,
  id: externalId,
}: FileUploadProps) {
  const internalId = useId();
  const inputId = externalId ?? `file-upload-${internalId}`;
  const inputRef = useRef<HTMLInputElement>(null);
  const [isDragging, setIsDragging] = useState(false);
  const [sizeError, setSizeError] = useState<string | null>(null);

  const validateAndAdd = useCallback(
    (incoming: FileList | File[]) => {
      const list = Array.from(incoming);
      setSizeError(null);

      if (maxSizeBytes) {
        const oversized = list.filter((f) => f.size > maxSizeBytes);
        if (oversized.length > 0) {
          setSizeError(
            `File${oversized.length > 1 ? "s" : ""} too large (max ${formatBytes(maxSizeBytes)}): ${oversized.map((f) => f.name).join(", ")}`
          );
          return;
        }
      }

      if (multiple) {
        // Merge with existing, deduplicate by name+size
        const merged = [...files];
        list.forEach((f) => {
          const dup = merged.find((e) => e.name === f.name && e.size === f.size);
          if (!dup) merged.push(f);
        });
        onFilesChange(merged);
      } else {
        const first = list[0];
        if (first) onFilesChange([first]);
      }
    },
    [files, maxSizeBytes, multiple, onFilesChange]
  );

  const removeFile = useCallback(
    (index: number) => {
      const next = files.filter((_, i) => i !== index);
      onFilesChange(next);
      setSizeError(null);
    },
    [files, onFilesChange]
  );

  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault();
    if (!disabled) setIsDragging(true);
  };

  const handleDragLeave = (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);
    if (disabled) return;
    if (e.dataTransfer.files.length > 0) {
      validateAndAdd(e.dataTransfer.files);
    }
  };

  const handleInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (e.target.files && e.target.files.length > 0) {
      validateAndAdd(e.target.files);
      // Reset input value so the same file can be re-selected after removal
      e.target.value = "";
    }
  };

  const displayError = error ?? sizeError;

  return (
    <div className={cn("space-y-2", className)}>
      {/* Drop zone */}
      <div
        role="button"
        aria-label={label}
        aria-disabled={disabled}
        tabIndex={disabled ? -1 : 0}
        onClick={() => !disabled && inputRef.current?.click()}
        onKeyDown={(e) => {
          if (!disabled && (e.key === "Enter" || e.key === " ")) {
            e.preventDefault();
            inputRef.current?.click();
          }
        }}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
        className={cn(
          "flex flex-col items-center justify-center gap-2 px-4 py-8",
          "rounded-lg border-2 border-dashed",
          "cursor-pointer transition-colors duration-150",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary",
          disabled
            ? "border-border-light bg-bg-secondary cursor-not-allowed opacity-60"
            : isDragging
              ? "border-primary bg-primary/5"
              : displayError
                ? "border-danger bg-danger/5"
                : "border-border hover:border-primary hover:bg-primary/5"
        )}
      >
        <span
          className={cn(
            "text-text-muted",
            isDragging && "text-primary",
            displayError && "text-danger"
          )}
        >
          <UploadIcon />
        </span>
        <div className="text-center">
          <p className="text-sm font-medium text-text-primary">
            {isDragging
              ? "Drop files here"
              : "Drag and drop files here, or click to browse"}
          </p>
          {(accept || maxSizeBytes) && (
            <p className="mt-1 text-xs text-text-muted">
              {[
                accept
                  ? `Accepted: ${accept}`
                  : null,
                maxSizeBytes
                  ? `Max size: ${formatBytes(maxSizeBytes)}`
                  : null,
              ]
                .filter(Boolean)
                .join(" · ")}
            </p>
          )}
        </div>
      </div>

      {/* Hidden file input */}
      <input
        ref={inputRef}
        id={inputId}
        type="file"
        accept={accept}
        multiple={multiple}
        disabled={disabled}
        onChange={handleInputChange}
        className="sr-only"
        tabIndex={-1}
        aria-hidden="true"
      />

      {/* File list */}
      {files.length > 0 && (
        <ul className="space-y-1" aria-label="Selected files">
          {files.map((file, index) => (
            <li
              key={`${file.name}-${file.size}-${index}`}
              className={cn(
                "flex items-center justify-between gap-2 px-3 py-2",
                "rounded-md bg-bg-secondary border border-border",
                "text-sm"
              )}
            >
              <span className="truncate text-text-primary" title={file.name}>
                {file.name}
              </span>
              <span className="shrink-0 text-xs text-text-muted">
                {formatBytes(file.size)}
              </span>
              {!disabled && (
                <button
                  type="button"
                  aria-label={`Remove ${file.name}`}
                  onClick={() => removeFile(index)}
                  className={cn(
                    "shrink-0 rounded p-0.5 text-text-muted",
                    "hover:bg-gray-200 hover:text-text-primary",
                    "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                  )}
                >
                  <RemoveIcon />
                </button>
              )}
            </li>
          ))}
        </ul>
      )}

      {/* Error */}
      {displayError && (
        <p role="alert" className="text-sm text-danger">
          {displayError}
        </p>
      )}

      {/* Hint */}
      {hint && !displayError && (
        <p className="text-sm text-text-muted">{hint}</p>
      )}
    </div>
  );
}
