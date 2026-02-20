// ============================================================
// /app/settings — Platform settings (placeholder)
// ============================================================
import { Settings } from 'lucide-react';

export default function SettingsPage() {
  return (
    <div data-testid="settings-page">
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-[--color-text-primary]">Settings</h1>
        <p className="text-sm text-[--color-text-secondary] mt-1">
          Platform configuration and preferences
        </p>
      </div>

      <div className="rounded-lg border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center">
        <Settings className="h-12 w-12 text-[--color-text-muted] mx-auto mb-4" />
        <h2 className="text-lg font-semibold text-[--color-text-primary] mb-2">
          Platform Settings
        </h2>
        <p className="text-sm text-[--color-text-secondary] max-w-md mx-auto mb-6">
          Configure platform-wide defaults, notification preferences, integration
          credentials, and tenant provisioning templates.
        </p>
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 max-w-lg mx-auto">
          <PlannedFeature title="General" description="Platform name, timezone, default currency" />
          <PlannedFeature title="Notifications" description="Email templates and alert thresholds" />
          <PlannedFeature title="Integrations" description="API keys and webhook configurations" />
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
