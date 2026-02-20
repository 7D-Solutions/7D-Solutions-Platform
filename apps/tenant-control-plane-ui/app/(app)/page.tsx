// ============================================================
// /app/ — redirect to /app/tenants (landing page)
// ============================================================
import { redirect } from 'next/navigation';

export default function AppIndexPage() {
  redirect('/tenants');
}
