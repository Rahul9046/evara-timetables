// `vitest/config` rather than `vite` so the `test` block below is type-checked.
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Tauri serves the frontend from a fixed port in development and from `dist/` in a
// release build. `clearScreen: false` keeps Rust compiler output visible during
// `tauri dev`, and ignoring `src-tauri` stops Rust rebuilds from triggering an HMR loop.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "es2022",
    sourcemap: true,
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    // Unmounts rendered components between tests. See src/test-setup.ts for why this is
    // not automatic here.
    setupFiles: ["./src/test-setup.ts"],
  },
});
