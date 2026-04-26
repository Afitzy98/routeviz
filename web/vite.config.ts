import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import wasm from "vite-plugin-wasm";
import path from "path";

// GitHub Pages serves this site under /<repo-name>/ — adjust if you fork or
// rename the repo, or override at build time with the VITE_BASE env var.
const base = process.env.VITE_BASE ?? "/routeviz/";

export default defineConfig(({ mode }) => ({
  base: mode === "production" ? base : "/",
  plugins: [react(), wasm()],
  server: {
    fs: {
      allow: [".", path.resolve(__dirname, "../wasm/pkg")],
    },
  },
}));
