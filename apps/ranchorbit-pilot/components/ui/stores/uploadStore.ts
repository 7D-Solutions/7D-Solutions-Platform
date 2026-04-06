type Subscriber = () => void;

export type UploadStatus = "pending" | "uploading" | "success" | "error";

export interface FileMetadata {
  name: string;
  size: number;
  type: string;
  lastModified: number;
  uploadStatus: UploadStatus;
  errorMessage?: string;
  uploadedAt?: number;
}

interface BucketState {
  files: Map<string, FileMetadata>;
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
    state.set(key, { files: new Map(), progress: new Map(), isUploading: false });
  }
  return state.get(key)!;
}

export const uploadStore = {
  subscribe(fn: Subscriber): () => void {
    subscribers.add(fn);
    return () => subscribers.delete(fn);
  },

  getSnapshot(): ReadonlyMap<string, Readonly<BucketState>> {
    return state;
  },

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

  removeFile(key: string, slot: string): void {
    const b = bucket(key);
    b.files.delete(slot);
    b.progress.delete(slot);
    notify();
  },

  setProgress(key: string, slot: string, progress: number): void {
    const b = bucket(key);
    b.progress.set(slot, Math.min(100, Math.max(0, progress)));
    const meta = b.files.get(slot);
    if (meta) b.files.set(slot, { ...meta, uploadStatus: "uploading" });
    notify();
  },

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

  setError(key: string, slot: string, message: string): void {
    const b = bucket(key);
    const meta = b.files.get(slot);
    if (meta) b.files.set(slot, { ...meta, uploadStatus: "error", errorMessage: message });
    notify();
  },

  setIsUploading(key: string, isUploading: boolean): void {
    bucket(key).isUploading = isUploading;
    notify();
  },

  clearAll(key: string): void {
    const b = bucket(key);
    b.files.clear();
    b.progress.clear();
    b.isUploading = false;
    notify();
  },

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

  isUploading(snap: ReadonlyMap<string, Readonly<BucketState>>, key: string): boolean {
    return snap.get(key)?.isUploading ?? false;
  },

  hasFiles(snap: ReadonlyMap<string, Readonly<BucketState>>, key: string): boolean {
    return (snap.get(key)?.files.size ?? 0) > 0;
  },
};
