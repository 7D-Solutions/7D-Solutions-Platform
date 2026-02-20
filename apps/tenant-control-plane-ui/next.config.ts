import type { NextConfig } from 'next';

const nextConfig: NextConfig = {
  // TCP UI is self-contained — no cross-app routing needed
  // All backend calls go through BFF (Next.js API routes)
  output: 'standalone',
};

export default nextConfig;
