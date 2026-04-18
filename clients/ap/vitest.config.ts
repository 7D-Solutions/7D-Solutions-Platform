import { defineConfig } from "vitest/config";
import { resolve } from "node:path";

export default defineConfig({
  test: {
    environment: "node",
    envDir: resolve(import.meta.dirname, "../.."),
  },
});
