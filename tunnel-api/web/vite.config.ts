import { resolve } from "node:path";
import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

export default defineConfig({
  root: import.meta.dirname,
  base: "/web/",
  plugins: [solid()],
  build: {
    outDir: resolve(import.meta.dirname, "../web-dist"),
    emptyOutDir: true,
  },
});
