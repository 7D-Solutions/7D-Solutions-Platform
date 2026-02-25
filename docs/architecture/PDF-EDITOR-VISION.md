# PDF Editor Module — Vision & Roadmap

**Version**: 0.1.0
**Last Updated**: 2026-02-25
**Status**: Active

---

## 1. Mission

PDF Editor provides **document generation and form management** for the platform. It manages PDF templates, renders documents from templates plus data, handles form field annotations, and processes form submissions. Other modules call PDF Editor via HTTP API when they need to produce printable documents (invoices, packing slips, reports).

### Non-Goals

PDF Editor does **NOT**:
- Own any business domain data (invoicing, shipping, etc.)
- Store business records beyond generated document metadata
- Handle document signing (future integration with e-signature providers)
- Manage non-PDF document formats

---

## 2. Domain Authority

| Domain Entity | PDF Editor Authority |
|---|---|
| **Form Templates** | Template definitions with field layouts |
| **Form Fields** | Per-template field positions, types, and validation rules |
| **Form Submissions** | Filled form data submissions (draft/submitted) |

Note: The editor is a **stateless processing engine** — it does not store PDF files or annotation data. Annotations live in the browser; generated PDFs are returned as response bytes.

---

## 3. Data Ownership

| Table | Purpose |
|---|---|
| `form_templates` | Form template definitions |
| `form_fields` | Per-template field positions, types, validation rules |
| `form_submissions` | Submitted form data records |
| `events_outbox` | Module outbox for NATS |

---

## 4. Events

**Produces (planned):**
- `pdf.form.submitted` — form submission transitions to submitted
- `pdf.document.generated` — processed PDF returned (from annotations or form fill)

**Consumes:** None (called via HTTP API by other modules)

---

## 5. Key Invariants

1. Form submissions use draft → submitted status machine (v1)
2. Form submissions are validated against template field definitions
3. Tenant isolation on every table and query
4. PDF files are never stored — editor receives bytes, processes, returns result

---

## 6. Integration Map

- **AR** → future: invoice PDF generation
- **AP** → future: PO and bill PDF generation
- **Shipping-Receiving** → future: packing slip PDF generation
- **Reporting** → future: report PDF export

---

## 7. Roadmap

### v0.1.0 (current)
- Template management CRUD
- Form field annotation configuration
- Document generation from template + data
- Form submission processing
- Generated document retrieval

### v1.0.0 (proven)
- Bulk document generation (batch invoices, statements)
- E-signature integration
- Template version management with migration
- Custom branding/theming per tenant
- Document storage integration (S3/cloud storage)
