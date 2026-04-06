"use client";

import { QueryClientProvider } from "@tanstack/react-query";
import { createQueryClient } from "@7d/platform-client";
import { useState } from "react";

export function Providers({ children }: { children: React.ReactNode }) {
  // One QueryClient instance per client-side session.
  const [queryClient] = useState(() => createQueryClient());
  return (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}
