import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { marked } from "marked";
import type { Plugin } from "vite";
import { captureHelp } from "./capture-help.ts";

const HELP_MODULE = "virtual:spawnshell-help";
const REPO_URL = "https://github.com/ErBlack/spawnshell";
const RELEASES_URL = `${REPO_URL}/releases`;
const DOWNLOAD_URL = `${RELEASES_URL}/latest/download/Spawnshell_universal.dmg`;

interface SiteOptions {
  help: { binary: string; json: string | undefined };
  version: string;
  macos: string;
  changelog: string;
  backend: { real: string; fake: string };
}

export function sitePlugin(options: SiteOptions): Plugin {
  return {
    name: "spawnshell-site",
    enforce: "pre",
    resolveId(source, importer) {
      if (source === HELP_MODULE) return `\0${HELP_MODULE}`;
      if (!importer) return;
      const target = resolve(dirname(importer.split("?")[0]), source);
      if (target === options.backend.real) return options.backend.fake;
    },
    load(id) {
      if (id !== `\0${HELP_MODULE}`) return;
      const { binary, json } = options.help;
      const help = json
        ? JSON.parse(readFileSync(json, "utf8"))
        : captureHelp(binary);
      return `export default ${JSON.stringify(help)};`;
    },
    transformIndexHtml: {
      order: "pre",
      handler(html) {
        return html
          .replaceAll("%VERSION%", options.version)
          .replaceAll("%DOWNLOAD%", DOWNLOAD_URL)
          .replaceAll("%RELEASES%", RELEASES_URL)
          .replaceAll("%REPO%", REPO_URL)
          .replaceAll("%MACOS%", options.macos)
          .replace("%CHANGELOG%", () =>
            marked.parse(readFileSync(options.changelog, "utf8"), {
              async: false,
            }),
          );
      },
    },
  };
}
