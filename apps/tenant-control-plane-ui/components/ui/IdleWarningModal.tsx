'use client';
// ============================================================
// IdleWarningModal — countdown + "Stay logged in" button
// Shown when user has been idle for (30 min - warning period)
// ============================================================
import { useEffect, useState } from 'react';
import { Modal } from './Modal';
import { Button } from './Button';

interface IdleWarningModalProps {
  isOpen: boolean;
  remainingMs: number;
  onStayLoggedIn: () => void;
  onLogout: () => void;
}

function formatRemaining(ms: number): string {
  const totalSeconds = Math.max(0, Math.ceil(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

export function IdleWarningModal({
  isOpen,
  remainingMs,
  onStayLoggedIn,
  onLogout,
}: IdleWarningModalProps) {
  const [displayMs, setDisplayMs] = useState(remainingMs);

  useEffect(() => {
    setDisplayMs(remainingMs);
  }, [remainingMs]);

  useEffect(() => {
    if (!isOpen) return;
    const interval = setInterval(() => {
      setDisplayMs((prev) => Math.max(0, prev - 1000));
    }, 1000);
    return () => clearInterval(interval);
  }, [isOpen]);

  return (
    <Modal
      isOpen={isOpen}
      title="Are you still there?"
      onClose={onStayLoggedIn}
      size="sm"
      preventClosing
    >
      <Modal.Body>
        <p className="text-sm text-[--color-text-secondary]">
          You've been inactive for a while. For your security, you'll be automatically logged out
          in:
        </p>
        <p className="mt-3 text-center text-4xl font-bold text-[--color-danger] tabular-nums">
          {formatRemaining(displayMs)}
        </p>
        <p className="mt-3 text-sm text-[--color-text-secondary]">
          Your work has been saved. You can log back in at any time.
        </p>
      </Modal.Body>
      <Modal.Actions>
        <Button variant="ghost" onClick={onLogout}>
          Log out now
        </Button>
        <Button variant="primary" onClick={onStayLoggedIn}>
          Stay logged in
        </Button>
      </Modal.Actions>
    </Modal>
  );
}
