import React, { useCallback, useRef, useState } from "react";

export interface Column {
  id: string;
  label: string;
  visible: boolean;
  locked?: boolean;
  align?: "left" | "center" | "right";
}

export interface ColumnManagerResult {
  columns: Column[];
  isEditMode: boolean;
  toggleEditMode: () => void;
  toggleVisibility: (id: string) => void;
  resetToDefault: () => void;
  getColumnVisibility: (id: string) => boolean;
  handleDragStart: (e: React.DragEvent<HTMLElement>, index: number) => void;
  handleDragOver: (e: React.DragEvent<HTMLElement>, index: number) => void;
  handleDrop: (e: React.DragEvent<HTMLElement>, index: number) => void;
  handleDragEnd: () => void;
  dragState: { draggedIndex: number | null; targetIndex: number | null };
}

export function useColumnManager(
  tableId: string,
  defaultColumns: Column[],
  options?: {
    onSave?: (columns: Column[]) => void;
  }
): ColumnManagerResult {
  const storageKey = `col-cfg-${tableId}`;

  const [originalDefaults] = useState<Column[]>(() =>
    defaultColumns.map((c) => ({ ...c, visible: c.visible ?? true }))
  );

  const [columns, setColumns] = useState<Column[]>(() => {
    try {
      const saved = localStorage.getItem(storageKey);
      if (saved) {
        const parsed = JSON.parse(saved) as Column[];
        const savedIds = parsed.map((c) => c.id).sort().join(",");
        const defaultIds = originalDefaults.map((c) => c.id).sort().join(",");
        if (savedIds === defaultIds) return parsed;
      }
    } catch {
      // quota exceeded or invalid JSON
    }
    return originalDefaults.map((c) => ({ ...c }));
  });

  const [isEditMode, setIsEditMode] = useState(false);
  const [pendingVisibility, setPendingVisibility] = useState<Record<string, boolean>>({});
  const [draggedIndex, setDraggedIndex] = useState<number | null>(null);
  const [targetIndex, setTargetIndex] = useState<number | null>(null);

  const save = useCallback(
    (cols: Column[]) => {
      try {
        localStorage.setItem(storageKey, JSON.stringify(cols));
      } catch {
        // non-fatal
      }
      options?.onSave?.(cols);
    },
    [storageKey, options]
  );

  const toggleEditMode = useCallback(() => {
    setIsEditMode((prev) => {
      if (prev) {
        if (Object.keys(pendingVisibility).length > 0) {
          const updated = columns.map((c) => ({
            ...c,
            visible: pendingVisibility[c.id] ?? c.visible,
          }));
          setColumns(updated);
          save(updated);
        }
        setPendingVisibility({});
      } else {
        const snap: Record<string, boolean> = {};
        columns.forEach((c) => { snap[c.id] = c.visible; });
        setPendingVisibility(snap);
      }
      return !prev;
    });
  }, [columns, pendingVisibility, save]);

  const toggleVisibility = useCallback(
    (id: string) => {
      const current = pendingVisibility[id] ?? columns.find((c) => c.id === id)?.visible ?? true;
      const next = !current;
      setPendingVisibility((prev) => ({ ...prev, [id]: next }));
      const updated = columns.map((c) => (c.id === id ? { ...c, visible: next } : c));
      setColumns(updated);
      save(updated);
    },
    [columns, pendingVisibility, save]
  );

  const resetToDefault = useCallback(() => {
    const reset = originalDefaults.map((c) => ({ ...c }));
    setColumns(reset);
    setPendingVisibility({});
    save(reset);
  }, [originalDefaults, save]);

  const getColumnVisibility = useCallback(
    (id: string) => {
      const pending = pendingVisibility[id];
      const actual = columns.find((c) => c.id === id)?.visible;
      return isEditMode ? (pending ?? actual ?? true) : (actual ?? true);
    },
    [isEditMode, pendingVisibility, columns]
  );

  const handleDragStart = useCallback(
    (e: React.DragEvent<HTMLElement>, index: number) => {
      setDraggedIndex(index);
      e.dataTransfer.effectAllowed = "move";
    },
    []
  );

  const handleDragOver = useCallback(
    (e: React.DragEvent<HTMLElement>, index: number) => {
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      if (draggedIndex !== null && draggedIndex !== index) setTargetIndex(index);
    },
    [draggedIndex]
  );

  const handleDrop = useCallback(
    (e: React.DragEvent<HTMLElement>, index: number) => {
      e.preventDefault();
      if (draggedIndex === null || draggedIndex === index) {
        setDraggedIndex(null);
        setTargetIndex(null);
        return;
      }
      const next = [...columns];
      const spliced = next.splice(draggedIndex, 1);
      const col = spliced[0];
      if (!col) { setDraggedIndex(null); setTargetIndex(null); return; }
      next.splice(index, 0, col);
      setColumns(next);
      save(next);
      setDraggedIndex(null);
      setTargetIndex(null);
    },
    [draggedIndex, columns, save]
  );

  const handleDragEnd = useCallback(() => {
    setDraggedIndex(null);
    setTargetIndex(null);
  }, []);

  const resultRef = useRef<ColumnManagerResult>({} as ColumnManagerResult);
  resultRef.current.columns = columns;
  resultRef.current.isEditMode = isEditMode;
  resultRef.current.toggleEditMode = toggleEditMode;
  resultRef.current.toggleVisibility = toggleVisibility;
  resultRef.current.resetToDefault = resetToDefault;
  resultRef.current.getColumnVisibility = getColumnVisibility;
  resultRef.current.handleDragStart = handleDragStart;
  resultRef.current.handleDragOver = handleDragOver;
  resultRef.current.handleDrop = handleDrop;
  resultRef.current.handleDragEnd = handleDragEnd;
  resultRef.current.dragState = { draggedIndex, targetIndex };

  return resultRef.current;
}
