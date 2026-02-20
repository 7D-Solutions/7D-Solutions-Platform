'use client';
// ============================================================
// StatusBadge — all status rendering goes through this component
// Rule: Never render status inline. Never hardcode colors.
// ============================================================
import { clsx } from 'clsx';

export type StatusAudience = 'admin' | 'driver' | 'customer';
export type BadgeVariant = 'default' | 'compact' | 'large';

interface StatusConfig {
  label: Record<StatusAudience, string>;
  colorClass: string;  // Tailwind-ish using CSS vars
}

const platformStatuses: Record<string, StatusConfig> = {
  active:      { label: { admin: 'Active',      driver: 'Active',      customer: 'Active'      }, colorClass: 'badge-success'  },
  suspended:   { label: { admin: 'Suspended',   driver: 'On Hold',     customer: 'Suspended'   }, colorClass: 'badge-warning'  },
  terminated:  { label: { admin: 'Terminated',  driver: 'Inactive',    customer: 'Closed'      }, colorClass: 'badge-danger'   },
  pending:     { label: { admin: 'Setting up',  driver: 'Setting up',  customer: 'Setting up'  }, colorClass: 'badge-info'     },
  past_due:    { label: { admin: 'Past Due',    driver: 'Past Due',    customer: 'Past Due'    }, colorClass: 'badge-danger'   },
  degraded:    { label: { admin: 'Degraded',    driver: 'Degraded',    customer: 'Service Issue'}, colorClass: 'badge-warning' },
  unknown:     { label: { admin: 'Unknown',     driver: 'Unknown',     customer: 'Unknown'     }, colorClass: 'badge-neutral'  },
  available:   { label: { admin: 'Available',   driver: 'Available',   customer: 'Available'   }, colorClass: 'badge-success'  },
  unavailable: { label: { admin: 'Unavailable', driver: 'Unavailable', customer: 'Unavailable' }, colorClass: 'badge-danger'   },
  trial:       { label: { admin: 'Trial',       driver: 'Trial',       customer: 'Trial'       }, colorClass: 'badge-info'     },
  cancelled:   { label: { admin: 'Cancelled',   driver: 'Cancelled',   customer: 'Cancelled'   }, colorClass: 'badge-neutral'  },
  overdue:     { label: { admin: 'Overdue',     driver: 'Overdue',     customer: 'Overdue'     }, colorClass: 'badge-danger'   },
  paid:        { label: { admin: 'Paid',        driver: 'Paid',        customer: 'Paid'        }, colorClass: 'badge-success'  },
  processing:  { label: { admin: 'Processing',  driver: 'Processing',  customer: 'Processing'  }, colorClass: 'badge-info'     },
};

// App-specific statuses can be added here
const appStatuses: Record<string, StatusConfig> = {};

const allStatuses = { ...platformStatuses, ...appStatuses };

const variantClasses: Record<BadgeVariant, string> = {
  default: 'px-2.5 py-0.5 text-xs font-medium rounded-full',
  compact: 'px-1.5 py-px text-[10px] font-medium rounded',
  large:   'px-3 py-1 text-sm font-semibold rounded-full',
};

const colorClasses: Record<string, string> = {
  'badge-success': 'bg-green-100 text-green-800',
  'badge-warning': 'bg-yellow-100 text-yellow-800',
  'badge-danger':  'bg-red-100 text-red-800',
  'badge-info':    'bg-blue-100 text-blue-800',
  'badge-neutral': 'bg-gray-100 text-gray-700',
};

export interface StatusBadgeProps {
  status: string;
  audience?: StatusAudience;
  variant?: BadgeVariant;
  className?: string;
}

export function StatusBadge({
  status,
  audience = 'admin',
  variant = 'default',
  className,
}: StatusBadgeProps) {
  const config = allStatuses[status.toLowerCase()];
  const label = config?.label[audience] ?? status;
  const colorClass = colorClasses[config?.colorClass ?? 'badge-neutral'] ?? colorClasses['badge-neutral'];

  return (
    <span
      className={clsx(variantClasses[variant], colorClass, className)}
      title={label}
    >
      {label}
    </span>
  );
}
