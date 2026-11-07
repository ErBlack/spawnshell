import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import project from "../package.json" with { type: "json" };
import tauri from "../src-tauri/tauri.conf.json" with { type: "json" };
import { sitePlugin } from "./build/site-plugin.ts";

const path = (relative: string) =>
  fileURLToPath(new URL(relative, import.meta.url));

export default defineConfig({
  root: path("."),
  base: "./",
  appType: "mpa",
  build: {
    outDir: path("dist"),
    emptyOutDir: true,
    sourcemap: true,
    rollupOptions: {
      input: {
        index: path("index.html"),
        changelog: path("changelog.html"),
        demo: path("demo.html"),
      },
    },
  },
  plugins: [
    sitePlugin({
      help: {
        binary:
          process.env.SPAWNSHELL_BIN ??
          path("../src-tauri/target/release/spawnshell"),
        json: process.env.SPAWNSHELL_HELP,
      },
      version: project.version,
      macos: tauri.bundle.macOS.minimumSystemVersion.replace(/\.0$/, ""),
      changelog: path("../CHANGELOG.md"),
      backend: {
        real: path("../src/backend.ts"),
        fake: path("src/demo/backend.ts"),
      },
    }),
  ],
});
