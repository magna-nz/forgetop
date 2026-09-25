import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// This project is deployed as a static GitHub Pages subdirectory. It deliberately
// has no development proxy or connection to the local forgetop dashboard server.
export default defineConfig({
  plugins: [react()],
  base: "/forgetop/demo/",
  build: { outDir: "dist", emptyOutDir: true },
});
