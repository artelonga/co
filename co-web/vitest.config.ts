import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "happy-dom",
    include: ["components/__tests__/**/*.test.ts"],
    exclude: ["e2e/**", "node_modules/**"],
    globals: false,
    setupFiles: ["components/__tests__/setup.ts"],
  },
});
