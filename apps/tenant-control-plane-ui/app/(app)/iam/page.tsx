// ============================================================
// /app/iam — Identity & Access Management (placeholder)
// ============================================================
import { Shield } from 'lucide-react';

export default function IamPage() {
  return (
    <div data-testid="iam-page">
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-[--color-text-primary]">Identity &amp; Access Management</h1>
        <p className="text-sm text-[--color-text-secondary] mt-1">
          Staff accounts, roles, and permission policies
        </p>
      </div>

      <div className="rounded-lg border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center">
        <Shield className="h-12 w-12 text-[--color-text-muted] mx-auto mb-4" />
        <h2 className="text-lg font-semibold text-[--color-text-primary] mb-2">
          Access Control
        </h2>
        <p className="text-sm text-[--color-text-secondary] max-w-md mx-auto mb-6">
          Manage platform staff accounts, assign roles with scoped permissions,
          and configure access policies for tenant control plane operations.
        </p>
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 max-w-lg mx-auto">
          <PlannedFeature title="Staff Users" description="Create, suspend, and manage staff accounts" />
          <PlannedFeature title="Roles" description="Define roles with granular permission sets" />
          <PlannedFeature title="Policies" description="IP allowlists, MFA requirements, session rules" />
        </div>
      </div>
    </div>
  );
}

function PlannedFeature({ title, description }: { title: string; description: string }) {
  return (
    <div className="rounded-[--radius-default] border border-dashed border-[--color-border-default] p-3">
      <p className="text-sm font-medium text-[--color-text-primary]">{title}</p>
      <p className="text-xs text-[--color-text-muted] mt-0.5">{description}</p>
    </div>
  );
}
