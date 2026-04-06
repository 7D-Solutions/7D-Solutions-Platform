"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { cn } from "@/components/ui/lib/cn";

const NAV_ITEMS = [
  {
    href: "/dashboard",
    label: "Dashboard",
    icon: (
      <svg width="18" height="18" viewBox="0 0 18 18" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
        <rect x="2" y="2" width="6" height="6" rx="1" />
        <rect x="10" y="2" width="6" height="6" rx="1" />
        <rect x="2" y="10" width="6" height="6" rx="1" />
        <rect x="10" y="10" width="6" height="6" rx="1" />
      </svg>
    ),
  },
  {
    href: "/animals",
    label: "Animals",
    icon: (
      <svg width="18" height="18" viewBox="0 0 18 18" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
        <circle cx="9" cy="7" r="3" />
        <path d="M3 16c0-3.314 2.686-6 6-6s6 2.686 6 6" />
        <path d="M2 5c0-1 .5-2 1.5-2s1.5 1 1.5 2" />
        <path d="M13 5c0-1 .5-2 1.5-2s1.5 1 1.5 2" />
      </svg>
    ),
  },
];

export default function AppLayout({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();

  return (
    <div className="flex min-h-screen bg-bg-primary">
      {/* Sidebar */}
      <aside className="w-56 shrink-0 border-r border-border bg-bg-secondary flex flex-col">
        {/* Brand header */}
        <div className="h-14 flex items-center gap-2.5 px-4 border-b border-border">
          <span className="text-primary font-bold text-xl tracking-tight">🐄</span>
          <span className="font-semibold text-text-primary text-sm">RanchOrbit</span>
        </div>

        {/* Nav */}
        <nav className="flex-1 px-2 py-3 space-y-0.5" aria-label="Main navigation">
          {NAV_ITEMS.map((item) => {
            const active = pathname === item.href || pathname.startsWith(item.href + "/");
            return (
              <Link
                key={item.href}
                href={item.href}
                className={cn(
                  "flex items-center gap-2.5 px-3 py-2 rounded-md text-sm font-medium transition-colors",
                  active
                    ? "bg-primary/10 text-primary"
                    : "text-text-secondary hover:bg-gray-100 hover:text-text-primary"
                )}
                aria-current={active ? "page" : undefined}
              >
                <span className={cn(active ? "text-primary" : "text-text-muted")}>
                  {item.icon}
                </span>
                {item.label}
              </Link>
            );
          })}
        </nav>

        {/* Footer */}
        <div className="px-4 py-3 border-t border-border">
          <p className="text-xs text-text-muted">RanchOrbit Pilot v0.1</p>
        </div>
      </aside>

      {/* Main content */}
      <div className="flex-1 flex flex-col min-w-0">
        <main className="flex-1 px-6 py-6">{children}</main>
      </div>
    </div>
  );
}
