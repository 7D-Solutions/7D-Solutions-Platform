// ============================================================
// Column management hook — visibility, drag-to-reorder, backend-persisted
// Port from: docs/reference/fireproof/src/infrastructure/hooks/useColumnManager.ts
// Adapted: no logger dependency; uses userPreferencesService for persistence.
// ============================================================
'use client';
import { useState, useEffect, useCallback, useRef } from 'react';
import { userPreferencesService } from '../services/userPreferencesService';
import { useActiveTabId } from '../state/tabStore';

export interface Column {
  id: string;
  label: string;
  visible: boolean;
  locked?: boolean;
  align?: 'left' | 'center' | 'right';
}

export interface UseColumnManagerReturn {
  columns: Column[];
  isEditMode: boolean;
  toggleEditMode: () => void;
  handleDragStart: (e: React.DragEvent<HTMLTableCellElement>, index: number) => void;
  handleDragOver: (e: React.DragEvent<HTMLTableCellElement>, index: number) => void;
  handleDrop: (e: React.DragEvent<HTMLTableCellElement>, index: number) => void;
  handleDragEnd: () => void;
  toggleVisibility: (id: string) => void;
  resetToDefault: () => void;
  getColumnVisibility: (columnId: string) => boolean;
  dragState: { draggedIndex: number | null; targetIndex: number | null };
}

/**
 * Column visibility, drag-to-reorder, and backend-persisted column configuration.
 * Tab-scoped: each tab has independent column customization.
 *
 * @example
 * const columnManager = useColumnManager('tenant-list', defaultColumns);
 */
export function useColumnManager(
  tableId: string,
  defaultColumns: Column[]
): UseColumnManagerReturn {
  const activeTabId = useActiveTabId() || 'default';
  const editModeKey = `table-edit-mode-${tableId}-${activeTabId}`;

  const [isEditMode, setIsEditMode] = useState(() => {
    if (typeof window === 'undefined') return false;
    return localStorage.getItem(editModeKey) === 'true';
  });
  const [columns, setColumns] = useState<Column[]>([]);
  const [draggedIndex, setDraggedIndex] = useState<number | null>(null);
  const [targetIndex, setTargetIndex] = useState<number | null>(null);
  const [pendingVisibility, setPendingVisibility] = useState<Record<string, boolean>>({});

  const [originalDefaults] = useState<Column[]>(
    defaultColumns.map((col) => ({ ...col, visible: col.visible !== undefined ? col.visible : true }))
  );

  useEffect(() => {
    if (typeof window !== 'undefined') {
      localStorage.setItem(editModeKey, String(isEditMode));
    }
  }, [isEditMode, editModeKey]);

  useEffect(() => {
    const preferenceKey = `column-config-${tableId}-tab-${activeTabId}`;
    let cancelled = false;

    const load = async () => {
      try {
        const saved = await userPreferencesService.getPreference<Column[]>(preferenceKey, null);
        if (cancelled) return;

        if (saved && Array.isArray(saved)) {
          const savedIds = saved.map((c) => c.id).sort();
          const defaultIds = originalDefaults.map((c) => c.id).sort();
          const idsMatch =
            savedIds.length === defaultIds.length &&
            savedIds.every((id, i) => id === defaultIds[i]);

          if (idsMatch) {
            setColumns(saved);
            return;
          }
        }
      } catch {
        // fall through to defaults
      }
      if (!cancelled) setColumns(originalDefaults);
    };

    load();
    return () => { cancelled = true; };
  }, [tableId, activeTabId, originalDefaults]);

  const saveConfig = useCallback(
    (newColumns: Column[]) => {
      userPreferencesService.savePreference(`column-config-${tableId}`, newColumns);
    },
    [tableId]
  );

  const toggleEditMode = useCallback(() => {
    setIsEditMode((prev) => {
      const next = !prev;
      if (!next && Object.keys(pendingVisibility).length > 0) {
        const updated = columns.map((col) => ({
          ...col,
          visible: pendingVisibility[col.id] ?? col.visible,
        }));
        setColumns(updated);
        saveConfig(updated);
        setPendingVisibility({});
      } else if (next) {
        const vis: Record<string, boolean> = {};
        columns.forEach((col) => { vis[col.id] = col.visible; });
        setPendingVisibility(vis);
      }
      return next;
    });
  }, [columns, pendingVisibility, saveConfig]);

  const toggleVisibility = useCallback(
    async (id: string) => {
      const current = pendingVisibility[id] ?? columns.find((c) => c.id === id)?.visible ?? true;
      const next = !current;
      setPendingVisibility((prev) => ({ ...prev, [id]: next }));
      const updated = columns.map((col) => (col.id === id ? { ...col, visible: next } : col));
      setColumns(updated);
      const preferenceKey = `column-config-${tableId}-tab-${activeTabId}`;
      await userPreferencesService.savePreference(preferenceKey, updated, true);
    },
    [columns, pendingVisibility, tableId, activeTabId]
  );

  const handleDragStart = useCallback(
    (e: React.DragEvent<HTMLTableCellElement>, index: number) => {
      setDraggedIndex(index);
      e.dataTransfer.effectAllowed = 'move';
    },
    []
  );

  const handleDragOver = useCallback(
    (e: React.DragEvent<HTMLTableCellElement>, index: number) => {
      e.preventDefault();
      e.dataTransfer.dropEffect = 'move';
      if (draggedIndex !== null && draggedIndex !== index) setTargetIndex(index);
    },
    [draggedIndex]
  );

  const handleDrop = useCallback(
    (e: React.DragEvent<HTMLTableCellElement>, index: number) => {
      e.preventDefault();
      if (draggedIndex === null || draggedIndex === index) {
        setDraggedIndex(null);
        setTargetIndex(null);
        return;
      }
      const newColumns = [...columns];
      const [dragged] = newColumns.splice(draggedIndex, 1);
      newColumns.splice(index, 0, dragged);
      setColumns(newColumns);
      saveConfig(newColumns);
      setDraggedIndex(null);
      setTargetIndex(null);
    },
    [draggedIndex, columns, saveConfig]
  );

  const handleDragEnd = useCallback(() => {
    setDraggedIndex(null);
    setTargetIndex(null);
  }, []);

  const resetToDefault = useCallback(async () => {
    const reset = originalDefaults.map((col) => ({ ...col }));
    setColumns(reset);
    setPendingVisibility({});
    const preferenceKey = `column-config-${tableId}-tab-${activeTabId}`;
    userPreferencesService.clearAllPending();
    await userPreferencesService.savePreference(preferenceKey, reset, true);
  }, [originalDefaults, tableId, activeTabId]);

  const getColumnVisibility = useCallback(
    (columnId: string) => {
      const pendingValue = pendingVisibility[columnId];
      const colValue = columns.find((c) => c.id === columnId)?.visible;
      return isEditMode ? (pendingValue ?? colValue ?? true) : (colValue ?? true);
    },
    [isEditMode, pendingVisibility, columns]
  );

  useEffect(() => {
    return () => { userPreferencesService.flushPendingSaves(); };
  }, []);

  const returnValueRef = useRef<UseColumnManagerReturn>({
    columns, isEditMode, toggleEditMode, handleDragStart, handleDragOver,
    handleDrop, handleDragEnd, toggleVisibility, resetToDefault, getColumnVisibility,
    dragState: { draggedIndex, targetIndex },
  });

  returnValueRef.current.columns = columns;
  returnValueRef.current.isEditMode = isEditMode;
  returnValueRef.current.toggleEditMode = toggleEditMode;
  returnValueRef.current.handleDragStart = handleDragStart;
  returnValueRef.current.handleDragOver = handleDragOver;
  returnValueRef.current.handleDrop = handleDrop;
  returnValueRef.current.handleDragEnd = handleDragEnd;
  returnValueRef.current.toggleVisibility = toggleVisibility;
  returnValueRef.current.resetToDefault = resetToDefault;
  returnValueRef.current.getColumnVisibility = getColumnVisibility;
  returnValueRef.current.dragState.draggedIndex = draggedIndex;
  returnValueRef.current.dragState.targetIndex = targetIndex;

  return returnValueRef.current;
}
