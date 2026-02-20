// ============================================================
// Formatters — all display formatting per PLATFORM-LANGUAGE.md
// Never format dates/currencies inline in components — use these.
// ============================================================

/**
 * Format a date value as "Jan 15, 2025" (no time).
 */
export function formatDate(value: string | Date | null | undefined): string {
  if (!value) return '—';
  try {
    const date = typeof value === 'string' ? new Date(value) : value;
    if (isNaN(date.getTime())) return '—';
    return date.toLocaleDateString('en-US', {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
    });
  } catch {
    return '—';
  }
}

/**
 * Format a date-time value as "Jan 15, 2025, 3:45 PM".
 */
export function formatDateTime(value: string | Date | null | undefined): string {
  if (!value) return '—';
  try {
    const date = typeof value === 'string' ? new Date(value) : value;
    if (isNaN(date.getTime())) return '—';
    return date.toLocaleDateString('en-US', {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      hour: 'numeric',
      minute: '2-digit',
    });
  } catch {
    return '—';
  }
}

/**
 * Format a number as USD currency: "$1,234.56"
 */
export function formatCurrency(
  value: number | string | null | undefined,
  currency = 'USD'
): string {
  if (value === null || value === undefined || value === '') return '—';
  const num = typeof value === 'string' ? parseFloat(value) : value;
  if (isNaN(num)) return '—';
  return new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency,
    minimumFractionDigits: 2,
  }).format(num);
}

/**
 * Format a decimal as a percentage: "12.5%"
 * Pass 0.125 for 12.5%, or 12.5 if already a percentage (set alreadyPercent=true).
 */
export function formatPercent(
  value: number | null | undefined,
  decimals = 1,
  alreadyPercent = false
): string {
  if (value === null || value === undefined) return '—';
  if (isNaN(value)) return '—';
  const pct = alreadyPercent ? value : value * 100;
  return `${pct.toFixed(decimals)}%`;
}

/**
 * Format a number with thousands separators: "1,234,567"
 */
export function formatNumber(
  value: number | string | null | undefined,
  decimals = 0
): string {
  if (value === null || value === undefined || value === '') return '—';
  const num = typeof value === 'string' ? parseFloat(value) : value;
  if (isNaN(num)) return '—';
  return new Intl.NumberFormat('en-US', {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  }).format(num);
}
