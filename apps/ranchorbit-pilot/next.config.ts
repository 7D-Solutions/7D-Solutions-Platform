import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Workspace packages ship TypeScript source — Next.js must transpile them.
  transpilePackages: ["@7d/platform-client", "@7d/tokens", "@7d/ui"],
};

export default nextConfig;
