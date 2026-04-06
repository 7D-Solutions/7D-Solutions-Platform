type Subscriber = () => void;

export type UploadStatus = "pending" | "uploading" | "success" | "error";

export interface FileMetadata {
  name: string;
  size: number;
  type: string;
  lastModified: number;
  uploadStatus: UploadStatus;
  /** Set when uploadStatus is 'error' */
  errorMessage?: string;
  /** Unix ms timestamp — set when uploadStatus is 'success' */
  uploadedAt?: number;
}

interface BucketState {
  /** Key is slot name, e.g. "certFile", "attachment-0" */
  files: Map<string, FileMetadata>;
  /** Upload progress per slot, 0–100 */
  progress: Map<string, number>;
  isUploading: boolean;
}

const state = new Map<string, BucketState>();
const subscribers = new Set<Subscriber>();

function notify(): void {
  subscribers.forEach((fn) => fn());
}

function bucket(key: string): BucketState {
  if (!state.has(key)) {
    state.set(key, {
      files: new Map(),
      progress: new Map(),
      isUploading: false,
    });
  }
  return state.get(key)!;
}

/**
 * Module-level upload store for file metadata and progress.
 * Compatible with React.useSyncExternalStore.
 *
 * File objects themselves are NOT stored here — keep them in component state
 * or refs. Only serialisable metadata lives here.
 *
 * Each "upload key" is an independent bucket — one per form/modal.
 *
 * @example
 *   const snap = React.useSyncExternalStore(uploadStore.subscribe, uploadStore.getSnapshot);
 *   const meta = uploadStore.getFile(snap, "cert-upload", "certFile");
 *   const progress = uploadStore.getProgress(snap, "cert-upload", "certFile");
 *
 *   // On file selected
 *   uploadStore.setFile("cert-upload", "certFile", file);
 *
 *   // During upload
 *   uploadStore.setProgress("cert-upload", "certFile", 60);
 *   uploadStore.markUploaded("cert-upload", "certFile");
 *
 *   // On error
 *   uploadStore.setError("cert-upload", "certFile", "File too large");
 */
export const uploadStore = {
  subscribe(fn: Subscriber): () => void {
    subscribers.add(fn);
    return () => subscribers.delete(fn);
  },

  getSnapshot(): ReadonlyMap<string, Readonly<BucketState>> {
    return state;
  },

  /** Set a file by slot. Extracts metadata; clears existing error. */
  setFile(key: string, slot: string, file: File): void {
    const b = bucket(key);
    b.files.set(slot, {
      name: file.name,
      size: file.size,
      type: file.type,
      lastModified: file.lastModified,
      uploadStatus: "pending",
    });
    b.progress.delete(slot);
    notify();
  },

  /** Remove a file slot and its progress/error. */
  removeFile(key: string, slot: string): void {
    const b = bucket(key);
    b.files.delete(slot);
    b.progress.delete(slot);
    notify();
  },

  /** Update upload progress for a slot (0–100). */
  setProgress(key: string, slot: string, progress: number): void {
    const b = bucket(key);
    b.progress.set(slot, Math.min(100, Math.max(0, progress)));
    const meta = b.files.get(slot);
    if (meta) {
      b.files.set(slot, { ...meta, uploadStatus: "uploading" });
    }
    notify();
  },

  /** Mark a slot as successfully uploaded. Sets progress to 100. */
  markUploaded(key: string, slot: string): void {
    const b = bucket(key);
    const meta = b.files.get(slot);
    if (meta) {
      b.files.set(slot, {
        ...meta,
        uploadStatus: "success",
        uploadedAt: Date.now(),
        errorMessage: undefined,
      });
    }
    b.progress.set(slot, 100);
    notify();
  },

  /** Record an upload error for a slot. */
  setError(key: string, slot: string, message: string): void {
    const b = bucket(key);
    const meta = b.files.get(slot);
    if (meta) {
      b.files.set(slot, { ...meta, uploadStatus: "error", errorMessage: message });
    }
    notify();
  },

  /** Set the global uploading flag for a bucket. */
  setIsUploading(key: string, isUploading: boolean): void {
    bucket(key).isUploading = isUploading;
    notify();
  },

  /** Remove all files and reset the bucket. */
  clearAll(key: string): void {
    const b = bucket(key);
    b.files.clear();
    b.progress.clear();
    b.isUploading = false;
    notify();
  },

  // ── Read helpers (work on the snapshot) ──

  getFile(
    snap: ReadonlyMap<string, Readonly<BucketState>>,
    key: string,
    slot: string
  ): FileMetadata | undefined {
    return snap.get(key)?.files.get(slot);
  },

  getProgress(
    snap: ReadonlyMap<string, Readonly<BucketState>>,
    key: string,
    slot: string
  ): number {
    return snap.get(key)?.progress.get(slot) ?? 0;
  },

  isUploading(
    snap: ReadonlyMap<string, Readonly<BucketState>>,
    key: string
  ): boolean {
    return snap.get(key)?.isUploading ?? false;
  },

  hasFiles(
    snap: ReadonlyMap<string, Readonly<BucketState>>,
    key: string
  ): boolean {
    return (snap.get(key)?.files.size ?? 0) > 0;
  },
};
