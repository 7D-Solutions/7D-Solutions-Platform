# PDF Editor Frontend — Integration Guide

This guide shows how to wire the platform's typed API client and Zustand stores
into an existing React application (e.g. `/Users/james/Projects/PDF-Creation/frontend/`).

## What Lives Here vs. In the React App

| Layer | Location | Description |
|-------|----------|-------------|
| Typed API client | `src/api/client.ts` | All HTTP calls to the Rust backend |
| TypeScript types | `src/api/types.ts` | Mirrors the Rust domain models |
| Zustand stores | `src/stores/*.ts` | Browser-local state (no server persistence for annotations) |
| Tests | `src/__tests__/*.test.ts` | Store smoke tests |

The React app (components, routing, CSS, PDF rendering) lives separately and imports
from here.

## Environment Variable

Set `VITE_PDF_API_BASE_URL` in the React app's `.env.local`:

```
VITE_PDF_API_BASE_URL=http://localhost:8102
```

The backend must set `CORS_ORIGINS` to include the frontend origin:
```
CORS_ORIGINS=http://localhost:5173
# or for development: CORS_ORIGINS=*
```

The `src/env.d.ts` file declares this variable for TypeScript. Copy it to the React
app's `src/` directory so Vite's `import.meta.env` type is correct.

## Importing the API Client

From the React app, import directly from the client:

```typescript
import { pdfApi, PdfApiError } from '<path-to-platform>/modules/pdf-editor/frontend/src/api';
import type { Annotation, FormTemplate } from '<path-to-platform>/modules/pdf-editor/frontend/src/api';
```

Or copy `src/api/client.ts`, `src/api/types.ts`, and `src/api/index.ts` into the
React app's `src/api/` directory (no symlinking required — these files have no
platform-specific imports).

## Save PDF with Annotations (the key wiring)

The Rust backend is **stateless** — no file upload, no server storage. Send the PDF
bytes and annotations on every save call.

```typescript
// In your save handler (e.g. App.tsx handleSavePDF):
import { pdfApi } from './api/client';

async function handleSavePDF(pdfBlobUrl: string, annotations: Annotation[]) {
  // 1. Fetch PDF bytes from the browser blob URL
  const response = await fetch(pdfBlobUrl);
  if (!response.ok) throw new Error('Failed to fetch PDF');
  const blob = await response.blob();

  // 2. Send bytes + annotations to the backend
  const annotatedBlob = await pdfApi.renderAnnotations(blob, annotations);

  // 3. Create a new blob URL for the result
  const downloadUrl = URL.createObjectURL(annotatedBlob);

  // 4. Show preview or trigger download
  showPdfPreview(downloadUrl, 'annotated.pdf');
}
```

### Replacing the Old Upload-Then-Stamp Pattern

The old Node.js backend required:
1. `POST /api/pdf/upload` → get back `{filename, path}`
2. `POST /api/pdf/stamp` with `{filename, stamps}` → get back `{stampedPath}`

The new Rust backend pattern:
1. `URL.createObjectURL(file)` — blob URL stays in the browser, no upload
2. `POST /api/pdf/render-annotations` — send bytes + annotations, get back bytes

**Tab store change:** The old `createTab(file, uploadData)` becomes `createTab(file)`.
The tab store creates the blob URL internally:

```typescript
// Old (upload-first pattern):
const data = await pdfApi.uploadPDF(file);       // upload to Node.js
createTab(file, { uploadedFilename: data.filename, pdfUrl: API_URL + data.path });

// New (stateless pattern):
createTab(file);   // blob URL created inside the store via URL.createObjectURL(file)
```

The `pdfTabStore` in `src/stores/pdfTabStore.ts` already implements this — `createTab`
takes only a `File` and creates the blob URL itself.

**uiStore change:** `openLocalPdf(file)` creates the blob URL in-place — no server call:

```typescript
// Old:
const data = await pdfApi.uploadPDF(file);
setPdfUrl(API_URL + data.path);
setUploadedFilename(data.filename);

// New:
useUIStore.getState().openLocalPdf(file);   // creates blob URL internally
```

## Migrating the Existing Frontend

For the PDF-Creation React app (`/Users/james/Projects/PDF-Creation/frontend/`):

### 1. Copy the API client

```bash
cp modules/pdf-editor/frontend/src/api/client.ts  /path/to/PDF-Creation/frontend/src/api/client.ts
cp modules/pdf-editor/frontend/src/api/types.ts   /path/to/PDF-Creation/frontend/src/api/types.ts
cp modules/pdf-editor/frontend/src/api/index.ts   /path/to/PDF-Creation/frontend/src/api/index.ts
cp modules/pdf-editor/frontend/src/env.d.ts        /path/to/PDF-Creation/frontend/src/env.d.ts
```

### 2. Update `.env`

```
VITE_API_URL=http://localhost:3002          # keep for any legacy paths
VITE_PDF_API_BASE_URL=http://localhost:8102  # new Rust backend
```

### 3. Replace `infrastructure/api/pdfApi.ts`

```typescript
// infrastructure/api/pdfApi.ts
import { pdfApi as typedApi } from '../../api/client';
import type { Annotation as ApiAnnotation } from '../../api/types';
import type { Annotation } from '../types';

export const pdfApi = {
  async renderAnnotations(pdfBlobUrl: string, annotations: Annotation[]): Promise<Blob> {
    const response = await fetch(pdfBlobUrl);
    if (!response.ok) {
      throw new Error(`Failed to fetch PDF: ${response.statusText}`);
    }
    const blob = await response.blob();
    // Local Annotation has extra fields (timestamp, textDecoration, etc.)
    // The Rust backend ignores unknown fields via serde's default behavior.
    return typedApi.renderAnnotations(blob, annotations as unknown as ApiAnnotation[]);
  },
};
```

### 4. Replace `usePdfOperations.ts` upload logic

```typescript
// infrastructure/hooks/usePdfOperations.ts
const uploadPdfFiles = async (files: File[]): Promise<void> => {
  if (files.length === 0) return;

  for (const file of files) {
    // No backend upload — create blob URL in the browser
    const blobUrl = URL.createObjectURL(file);
    createTab(file, { uploadedFilename: file.name, pdfUrl: blobUrl });

    if (file === files[0]) {
      setUploadedFilename(file.name);
      setPdfUrl(blobUrl);
      setStampedPdfUrl('');
      // Form detection not available in the stateless Rust backend
      setHasForm(false);
      setFormFields([]);
    }
  }
};
```

### 5. Replace `handleSavePDF` in `App.tsx`

```typescript
// App.tsx
const handleSavePDF = async () => {
  if (!activeTab || !activeTab.pdfUrl || annotations.length === 0) {
    notify.warning('Please add at least one annotation before saving');
    return;
  }

  setIsProcessing(true);
  try {
    const annotatedBlob = await pdfApi.renderAnnotations(activeTab.pdfUrl, annotations);
    const downloadUrl = URL.createObjectURL(annotatedBlob);
    setStampedPdfUrl(downloadUrl);
    const filename = `annotated_${activeTab.filename || 'document.pdf'}`;
    showPdfPreview(downloadUrl, filename);
    notify.success('PDF processed successfully! Review the preview.');
    updateActiveTab({ isDirty: false });
  } catch (error) {
    console.error('Save error:', error);
    notify.error('Failed to save PDF. Please try again.');
  } finally {
    setIsProcessing(false);
  }
};
```

## Form Template / Submission CRUD

The `formStore` (`src/stores/formStore.ts`) provides all template/submission CRUD.
It calls the typed API client directly. To use it in the React app:

```typescript
import { useFormStore } from '<path>/stores/formStore';

function FormPanel({ tenantId }: { tenantId: string }) {
  const { templates, loadTemplates, createTemplate } = useFormStore();

  useEffect(() => {
    loadTemplates(tenantId);
  }, [tenantId]);

  // ...
}
```

## Running Tests

```bash
cd modules/pdf-editor/frontend
npm test     # 43 tests across 6 files
```

Tests are pure store unit tests — no backend required. They run in under 200ms.
