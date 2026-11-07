import { readFileSync } from "node:fs";
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Shown next to the logo; the version lives in package.json (kept in sync with tauri.conf.json).
const pkg = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8"));

export default defineConfig({
  plugins: [svelte()],
  define: { __APP_VERSION__: JSON.stringify(pkg.version) },
  clearScreen: false,
  // The logo lives in the assets folder of the repository.
  server: { port: 5173, strictPort: true, fs: { allow: [".."] } },
});
