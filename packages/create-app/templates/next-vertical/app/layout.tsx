import type { Metadata } from "next";
// Design tokens: CSS custom properties that back the Tailwind theme.
import "@7d/tokens/tokens.css";
// Brand palette — swap this to change the entire colour scheme.
import "@7d/tokens/themes/__BRAND_THEME__";
import "./globals.css";
import { Providers } from "./providers";

export const metadata: Metadata = {
  title: "__APP_TITLE__",
  description: "__APP_TITLE__ — powered by 7D Solutions Platform",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" data-brand="__BRAND_THEME__">
      <body className="bg-bg-primary text-text-primary antialiased">
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}
