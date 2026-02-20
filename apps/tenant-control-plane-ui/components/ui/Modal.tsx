'use client';
// ============================================================
// Modal — portal rendering, no backdrop close, Escape key close
// Rules: Never window.confirm(). Always use this for dialogs.
// ============================================================
import { useEffect, useCallback } from 'react';
import { createPortal } from 'react-dom';
import { clsx } from 'clsx';
import { X } from 'lucide-react';

export type ModalSize = 'sm' | 'md' | 'lg' | 'xl';

const sizeWidth: Record<ModalSize, string> = {
  sm: 'max-w-[--modal-width-sm]',
  md: 'max-w-[--modal-width-md]',
  lg: 'max-w-[--modal-width-lg]',
  xl: 'max-w-[--modal-width-xl]',
};

interface ModalProps {
  isOpen: boolean;
  title: string;
  onClose: () => void;
  onFullClose?: () => void;
  size?: ModalSize;
  preventClosing?: boolean;
  children: React.ReactNode;
  className?: string;
}

function ModalBody({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <div
      className={clsx('flex-1 overflow-y-auto p-6', className)}
      style={{ maxHeight: 'var(--modal-content-max-height)' }}
    >
      {children}
    </div>
  );
}

function ModalActions({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <div
      className={clsx(
        'flex items-center justify-end gap-3 border-t border-[--color-border-light] px-6 py-4',
        className
      )}
      style={{ minHeight: 'var(--modal-footer-height)' }}
    >
      {children}
    </div>
  );
}

function ModalComponent({
  isOpen,
  title,
  onClose,
  onFullClose,
  size = 'md',
  preventClosing = false,
  children,
  className,
}: ModalProps) {
  const handleEscape = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !preventClosing) {
        onClose();
      }
    },
    [onClose, preventClosing]
  );

  useEffect(() => {
    if (!isOpen) return;
    document.addEventListener('keydown', handleEscape);
    document.body.style.overflow = 'hidden';
    return () => {
      document.removeEventListener('keydown', handleEscape);
      document.body.style.overflow = '';
    };
  }, [isOpen, handleEscape]);

  if (!isOpen) return null;

  const content = (
    <div
      className="fixed inset-0 flex items-center justify-center p-4"
      style={{ zIndex: 'var(--z-modal)' }}
      role="dialog"
      aria-modal="true"
      aria-labelledby="modal-title"
    >
      {/* Backdrop — no click-to-close */}
      <div
        className="absolute inset-0 bg-black/50"
        style={{ zIndex: 'var(--z-modal-backdrop)' }}
      />

      {/* Panel */}
      <div
        className={clsx(
          'relative z-10 flex flex-col w-full rounded-[--radius-lg] bg-[--color-bg-primary]',
          'shadow-[--shadow-2xl]',
          sizeWidth[size],
          className
        )}
        style={{ maxHeight: 'var(--modal-max-height)' }}
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-[--color-border-light] px-6 py-4">
          <h2
            id="modal-title"
            className="text-[--font-size-lg] font-[--font-weight-semibold] text-[--color-text-primary]"
          >
            {title}
          </h2>
          {!preventClosing && (
            <button
              onClick={onFullClose ?? onClose}
              className="rounded-[--radius-default] p-1 text-[--color-text-secondary] hover:bg-[--color-bg-tertiary] hover:text-[--color-text-primary] transition-[--transition-fast]"
              aria-label="Close"
            >
              <X className="h-5 w-5" />
            </button>
          )}
        </div>

        {/* Content */}
        {children}
      </div>
    </div>
  );

  if (typeof document === 'undefined') return null;
  return createPortal(content, document.body);
}

export const Modal = Object.assign(ModalComponent, {
  Body: ModalBody,
  Actions: ModalActions,
});
