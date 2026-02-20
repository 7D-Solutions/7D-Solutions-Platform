// ============================================================
// Tab-aware modal management hook
// Port from: docs/reference/fireproof/src/infrastructure/state/useTabModal.ts
// Adapted: uses next/navigation instead of react-router-dom
// ============================================================
'use client';
import { usePathname } from 'next/navigation';
import { useModalActions } from './modalStore';
import { useTabActions, useActiveTabId } from './tabStore';

export interface TabModalActions {
  openModal: (modalId: string, type: string, props: Record<string, unknown>) => void;
  closeModal: (modalId: string) => void;
  updateModalProps: (modalId: string, props: Record<string, unknown>) => void;
}

/**
 * Tab-aware modal hook. Always use this instead of useModalStore directly.
 * Automatically promotes preview tabs when a modal is opened.
 *
 * @example
 * const { openModal, closeModal } = useTabModal();
 * openModal('SUSPEND_TENANT', 'SUSPEND', { tenantId: tenant.id });
 */
export function useTabModal(): TabModalActions {
  const { openModal: openModalRaw, closeModal, updateModalProps } = useModalActions();
  const { promotePreviewTab, getTab } = useTabActions();
  const pathname = usePathname();
  const activeTabId = useActiveTabId();

  const openModal = (
    modalId: string,
    type: string,
    props: Record<string, unknown>
  ) => {
    const currentTab = getTab(activeTabId);
    if (currentTab?.isPreview) {
      promotePreviewTab(activeTabId);
    }
    openModalRaw(activeTabId, modalId, type, props, pathname ?? '/');
  };

  return { openModal, closeModal, updateModalProps };
}
