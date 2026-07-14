import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// `base: "./"` emits relative asset URLs so the bundle works no matter what path the Rust
// server mounts it at. In dev, `/api` is proxied to the running forgetop dashboard server.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  base: "./",
  // build.rs passes the crate version through; falls back to "dev" for a bare `npm run build`.
  define: { __APP_VERSION__: JSON.stringify(process.env.FORGETOP_VERSION ?? "dev") },
  build: { outDir: "dist", emptyOutDir: true },
  server: {
    port: 5250,
    proxy: { "/api": "http://127.0.0.1:8177" },
  },
});
