import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Falls back to reading the workspace version straight out of Cargo.toml so `npm run dev` (and
// a bare `npm run build`) show the real version too, instead of a "dev" placeholder — mirrors
// how the TUI bakes in `env!("CARGO_PKG_VERSION")` regardless of build mode.
function workspaceVersion() {
  const cargoToml = readFileSync(fileURLToPath(new URL("../../../Cargo.toml", import.meta.url)), "utf-8");
  const match = cargoToml.match(/^version\s*=\s*"([^"]+)"/m);
  if (!match) throw new Error("could not find workspace version in Cargo.toml");
  return match[1];
}

// `base: "./"` emits relative asset URLs so the bundle works no matter what path the Rust
// server mounts it at. In dev, `/api` is proxied to the running forgetop dashboard server.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  base: "./",
  // build.rs passes the crate version through during a real cargo build.
  define: { __APP_VERSION__: JSON.stringify(process.env.FORGETOP_VERSION ?? workspaceVersion()) },
  build: { outDir: "dist", emptyOutDir: true },
  server: {
    port: 5250,
    proxy: { "/api": "http://127.0.0.1:8177" },
  },
});
