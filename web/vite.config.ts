import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  base: "./",
  server: {
    port: 5174,
    strictPort: true,
  },
  build: {
    outDir: "dist",
  },
  // The real spartan-buffer-wasm .wasm binary (see package.json's
  // build:wasm script) is loaded via a normal Vite asset import --
  // no special-casing needed, Vite already handles .wasm as a real
  // fetchable asset URL.
});
