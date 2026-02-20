// ============================================================
// Upload state store factory — tab-scoped, localStorage-persisted
// Tracks file upload metadata and progress.
// ============================================================
'use client';
import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import { useActiveTabId } from './tabStore';

export interface UploadFile {
  name: string;
  size: number;
  type: string;
  progress: number; // 0-100
  status: 'pending' | 'uploading' | 'done' | 'error';
  error?: string;
  uploadedUrl?: string;
}

interface UploadStoreState {
  files: Record<string, UploadFile>;

  setFile: (fileId: string, file: UploadFile) => void;
  updateProgress: (fileId: string, progress: number) => void;
  markAsUploaded: (fileId: string, url: string) => void;
  setUploadError: (fileId: string, error: string) => void;
  removeFile: (fileId: string) => void;
  clearFiles: () => void;
}

const storeCache = new Map<string, ReturnType<typeof create>>();

/**
 * Tab-scoped, persistent upload state factory.
 *
 * @example
 * const { files, setFile, updateProgress, markAsUploaded } = useUploadStore('document-upload');
 */
export function useUploadStore(uploadKey: string) {
  const activeTabId = useActiveTabId();
  const storageKey = `upload-${uploadKey}-${activeTabId}`;

  let store: ReturnType<typeof create<UploadStoreState>>;
  if (storeCache.has(storageKey)) {
    store = storeCache.get(storageKey) as ReturnType<typeof create<UploadStoreState>>;
  } else {
    store = create<UploadStoreState>()(
      persist(
        (set) => ({
          files: {},

          setFile: (fileId, file) =>
            set((state) => ({ files: { ...state.files, [fileId]: file } })),

          updateProgress: (fileId, progress) =>
            set((state) => ({
              files: {
                ...state.files,
                [fileId]: state.files[fileId]
                  ? { ...state.files[fileId], progress, status: 'uploading' }
                  : state.files[fileId],
              },
            })),

          markAsUploaded: (fileId, url) =>
            set((state) => ({
              files: {
                ...state.files,
                [fileId]: state.files[fileId]
                  ? { ...state.files[fileId], progress: 100, status: 'done', uploadedUrl: url }
                  : state.files[fileId],
              },
            })),

          setUploadError: (fileId, error) =>
            set((state) => ({
              files: {
                ...state.files,
                [fileId]: state.files[fileId]
                  ? { ...state.files[fileId], status: 'error', error }
                  : state.files[fileId],
              },
            })),

          removeFile: (fileId) =>
            set((state) => {
              const { [fileId]: _removed, ...rest } = state.files;
              return { files: rest };
            }),

          clearFiles: () => set({ files: {} }),
        }),
        {
          name: storageKey,
          storage: createJSONStorage(() => localStorage),
          partialize: (state) => ({ files: state.files }),
        }
      )
    );
    storeCache.set(storageKey, store);
  }

  return store();
}
