import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Component tests run under jsdom against a mocked `fetch`/opener layer (see src/test/).
export default defineConfig({
  plugins: [react()],
  define: { __APP_VERSION__: JSON.stringify("test") },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
    css: false,
  },
});
