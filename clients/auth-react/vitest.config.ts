import { defineConfig } from "vitest/config";
import { resolve } from "node:path";

export default defineConfig({
  esbuild: {
    jsx: "automatic",
    jsxImportSource: "react",
  },
  test: {
    environment: "jsdom",
    globalSetup: ["./tests/global-setup.ts"],
    // Load .env from the project root (two levels up from clients/auth-react/).
    envDir: resolve(__dirname, "../../"),
  },
});
