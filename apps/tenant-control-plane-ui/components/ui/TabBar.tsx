// ============================================================
// TabBar — browser-style tab bar with preview, dirty, close confirm
// Port from: docs/reference/fireproof TabManager/TabBar + Tab
// Adapted: Tailwind + CSS tokens, platform Modal for confirm
// ============================================================
'use client';
import { useState, useRef, useEffect, useCallback } from 'react';
import { X, SplitSquareHorizontal } from 'lucide-react';
import { clsx } from 'clsx';
import {
  useTabs,
  useActiveTabId,
  useTabActions,
} from '@/infrastructure/state/tabStore';
import type { TabData } from '@/infrastructure/state/tabStore';
import { Modal } from './Modal';
import { Button } from './Button';

// ── Single Tab ──────────────────────────────────────────────

interface TabItemProps {
  tab: TabData;
  isActive: boolean;
  onClose: (tabId: string) => void;
  onContextMenu: (e: React.MouseEvent, tabId: string) => void;
}

function TabItem({ tab, isActive, onClose, onContextMenu }: TabItemProps) {
  const { setActiveTab, promotePreviewTab } = useTabActions();

  const handleClick = () => setActiveTab(tab.id);

  const handleDoubleClick = () => {
    if (tab.isPreview) promotePreviewTab(tab.id);
  };

  const handleClose = (e: React.MouseEvent) => {
    e.stopPropagation();
    onClose(tab.id);
  };

  return (
    <div
      role="tab"
      aria-selected={isActive}
      tabIndex={isActive ? 0 : -1}
      onClick={handleClick}
      onDoubleClick={handleDoubleClick}
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu(e, tab.id);
      }}
      title={tab.isPreview ? 'Double-click to keep this tab open' : undefined}
      data-testid={`tab-${tab.id}`}
      data-tab-preview={tab.isPreview ? 'true' : undefined}
      data-tab-dirty={tab.isDirty ? 'true' : undefined}
      className={clsx(
        'group relative flex items-center gap-1.5 px-3 cursor-pointer select-none',
        'border-r border-[--color-border-light] transition-[--transition-fast]',
        'min-w-[120px] max-w-[200px]',
        isActive
          ? 'bg-[--color-bg-primary] text-[--color-text-primary]'
          : 'bg-[--color-bg-secondary] text-[--color-text-secondary] hover:bg-[--color-bg-tertiary]'
      )}
      style={{ height: 'var(--tab-bar-height)' }}
    >
      {/* Active tab bottom border indicator */}
      {isActive && (
        <div className="absolute bottom-0 left-0 right-0 h-0.5 bg-[--color-primary]" />
      )}

      {/* Title */}
      <span
        className={clsx(
          'flex-1 truncate text-sm leading-tight',
          tab.isPreview && 'italic'
        )}
      >
        {tab.title}
      </span>

      {/* Dirty indicator */}
      {tab.isDirty && (
        <span
          className="text-[--color-primary] text-xs font-bold flex-shrink-0"
          data-testid={`dirty-indicator-${tab.id}`}
          aria-label="Unsaved changes"
        >
          &bull;
        </span>
      )}

      {/* Close button */}
      {tab.closeable !== false && (
        <button
          type="button"
          onClick={handleClose}
          className={clsx(
            'flex-shrink-0 rounded-[--radius-sm] p-0.5',
            'text-[--color-text-muted] hover:text-[--color-text-primary] hover:bg-[--color-bg-tertiary]',
            'opacity-0 group-hover:opacity-100 transition-[--transition-fast]',
            isActive && 'opacity-100'
          )}
          aria-label={`Close ${tab.title}`}
          data-testid={`close-tab-${tab.id}`}
        >
          <X className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  );
}

// ── Context Menu ────────────────────────────────────────────

interface ContextMenuState {
  visible: boolean;
  x: number;
  y: number;
  tabId: string | null;
}

const INITIAL_CONTEXT: ContextMenuState = { visible: false, x: 0, y: 0, tabId: null };

// ── TabBar ──────────────────────────────────────────────────

export function TabBar() {
  const tabs = useTabs();
  const activeTabId = useActiveTabId();
  const { closeTab, closeOtherTabs, enableSplitView } = useTabActions();

  // Dirty close confirmation
  const [confirmTabId, setConfirmTabId] = useState<string | null>(null);
  const confirmTab = confirmTabId ? tabs.find((t) => t.id === confirmTabId) : null;

  // Context menu
  const [ctx, setCtx] = useState<ContextMenuState>(INITIAL_CONTEXT);
  const ctxRef = useRef<HTMLDivElement>(null);

  // Close context menu on outside click
  useEffect(() => {
    if (!ctx.visible) return;
    const handle = (e: MouseEvent) => {
      if (ctxRef.current && !ctxRef.current.contains(e.target as Node)) {
        setCtx(INITIAL_CONTEXT);
      }
    };
    document.addEventListener('mousedown', handle);
    return () => document.removeEventListener('mousedown', handle);
  }, [ctx.visible]);

  const handleTabClose = useCallback(
    (tabId: string) => {
      const tab = tabs.find((t) => t.id === tabId);
      if (!tab) return;
      if (tab.isDirty) {
        setConfirmTabId(tabId);
      } else {
        closeTab(tabId);
      }
    },
    [tabs, closeTab]
  );

  const handleConfirmClose = () => {
    if (confirmTabId) closeTab(confirmTabId);
    setConfirmTabId(null);
  };

  const handleCancelClose = () => setConfirmTabId(null);

  const handleContextMenu = (e: React.MouseEvent, tabId: string) => {
    setCtx({ visible: true, x: e.clientX, y: e.clientY, tabId });
  };

  const handleSplitRight = () => {
    if (ctx.tabId) enableSplitView(activeTabId, ctx.tabId);
    setCtx(INITIAL_CONTEXT);
  };

  const handleCtxClose = () => {
    if (ctx.tabId) handleTabClose(ctx.tabId);
    setCtx(INITIAL_CONTEXT);
  };

  const handleCtxCloseOthers = () => {
    if (ctx.tabId) closeOtherTabs(ctx.tabId);
    setCtx(INITIAL_CONTEXT);
  };

  return (
    <>
      <div
        className="flex items-stretch border-b border-[--color-border-light] bg-[--color-bg-secondary] overflow-x-auto hide-scrollbar"
        style={{ height: 'var(--tab-bar-height)' }}
        role="tablist"
        data-testid="tab-bar"
      >
        {tabs.map((tab) => (
          <TabItem
            key={tab.id}
            tab={tab}
            isActive={tab.id === activeTabId}
            onClose={handleTabClose}
            onContextMenu={handleContextMenu}
          />
        ))}
      </div>

      {/* Context menu */}
      {ctx.visible && (
        <div
          ref={ctxRef}
          className="fixed bg-[--color-bg-primary] border border-[--color-border-default] rounded-[--radius-default] shadow-[--shadow-lg] py-1 min-w-[160px]"
          style={{ left: ctx.x, top: ctx.y, zIndex: 'var(--z-dropdown)' }}
          data-testid="tab-context-menu"
        >
          <button
            className="flex items-center gap-2 w-full px-3 py-1.5 text-sm text-[--color-text-primary] hover:bg-[--color-bg-secondary] text-left"
            onClick={handleSplitRight}
          >
            <SplitSquareHorizontal className="h-3.5 w-3.5" />
            Split Right
          </button>
          <div className="border-t border-[--color-border-light] my-1" />
          <button
            className="flex items-center gap-2 w-full px-3 py-1.5 text-sm text-[--color-text-primary] hover:bg-[--color-bg-secondary] text-left"
            onClick={handleCtxClose}
          >
            <X className="h-3.5 w-3.5" />
            Close Tab
          </button>
          <button
            className="flex items-center gap-2 w-full px-3 py-1.5 text-sm text-[--color-text-primary] hover:bg-[--color-bg-secondary] text-left"
            onClick={handleCtxCloseOthers}
          >
            <X className="h-3.5 w-3.5" />
            Close Other Tabs
          </button>
        </div>
      )}

      {/* Dirty-close confirmation modal */}
      <Modal
        isOpen={!!confirmTabId}
        title="Unsaved Changes"
        onClose={handleCancelClose}
        preventClosing
        size="sm"
      >
        <Modal.Body>
          <p className="text-sm text-[--color-text-primary]">
            &ldquo;{confirmTab?.title}&rdquo; has unsaved changes. Close anyway?
          </p>
        </Modal.Body>
        <Modal.Actions>
          <Button
            variant="ghost"
            size="sm"
            onClick={handleCancelClose}
            data-testid="dirty-cancel"
          >
            Cancel
          </Button>
          <Button
            variant="danger"
            size="sm"
            onClick={handleConfirmClose}
            data-testid="dirty-confirm"
          >
            Close Tab
          </Button>
        </Modal.Actions>
      </Modal>
    </>
  );
}
