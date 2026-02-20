// ============================================================
// TCP UI — Named Constants
// All magic numbers live here. Never hardcode these in components.
// ============================================================

// Pagination
export const PAGINATION_DEFAULT_PAGE_SIZE = 25;
export const PAGINATION_MIN_PAGE_SIZE = 10;
export const PAGINATION_MAX_PAGE_SIZE = 100;

// Search
export const SEARCH_DEBOUNCE_MS = 300;

// Toasts
export const TOAST_DURATION_MS = 4000;

// TanStack Query polling
export const REFETCH_INTERVAL_MS = 30_000;

// Idle timeout (30 minutes for staff console)
export const IDLE_TIMEOUT_MS = 30 * 60 * 1000;
export const IDLE_WARNING_MS = 5 * 60 * 1000;   // 5-minute warning before logout

// Support session polling
export const SUPPORT_SESSION_POLL_MS = 30_000;

// Button double-click protection
export const BUTTON_COOLDOWN_MS = 1000;

// Auth cookie name
export const AUTH_COOKIE_NAME = 'tcp_auth_token';

// Required role for TCP access
export const REQUIRED_ROLE = 'platform_admin';

// Backend service base URLs (used ONLY in BFF routes — never in browser code)
// These are set via environment variables
export const IDENTITY_AUTH_BASE_URL = process.env.IDENTITY_AUTH_BASE_URL ?? 'http://localhost:8090';
export const TENANT_REGISTRY_BASE_URL = process.env.TENANT_REGISTRY_BASE_URL ?? 'http://localhost:8091';
export const AR_BASE_URL = process.env.AR_BASE_URL ?? 'http://localhost:8080';
export const TTP_BASE_URL = process.env.TTP_BASE_URL ?? 'http://localhost:8095';
export const NOTIFICATIONS_BASE_URL = process.env.NOTIFICATIONS_BASE_URL ?? 'http://localhost:8094';

// Notification polling interval (same as general refetch)
export const NOTIFICATION_POLL_MS = 30_000;

// Audit service URL (server-side only)
export const AUDIT_SERVICE_BASE_URL = process.env.AUDIT_SERVICE_BASE_URL ?? 'http://localhost:8096';

// Audit max page size — hard ceiling for BFF validation
export const AUDIT_MAX_PAGE_SIZE = 100;
