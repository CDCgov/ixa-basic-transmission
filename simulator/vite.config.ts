import { defineConfig, type Plugin } from "vite";
import vue from "@vitejs/plugin-vue";
import { cfasimWasm } from "@cfasim-ui/wasm/vite";
import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { parse as parseToml } from "smol-toml";

// Reads `../config/*.{toml,json}` and emits a `virtual:presets` module so
// the frontend can list parameter presets. Each file has top-level
// `name`/`description` plus a `parameters` table.
function presetsPlugin(): Plugin {
  const id = "virtual:presets";
  const resolvedId = "\0" + id;
  const dir = resolve(import.meta.dirname, "..", "config");
  const isPresetFile = (f: string) => /\.(toml|json)$/i.test(f);
  return {
    name: "ixa-basic-transmission-presets",
    resolveId(source) {
      if (source === id) return resolvedId;
    },
    load(loadId) {
      if (loadId !== resolvedId) return;
      const files = readdirSync(dir).filter(isPresetFile).sort();
      const presets = files.map((f) => {
        const text = readFileSync(resolve(dir, f), "utf-8");
        const data = f.toLowerCase().endsWith(".json")
          ? JSON.parse(text)
          : parseToml(text);
        return { id: f.replace(/\.(toml|json)$/i, ""), ...data };
      });
      return `export default ${JSON.stringify(presets)};`;
    },
    configureServer(server) {
      server.watcher.add(dir);
      const reload = (file: string) => {
        if (!file.startsWith(dir) || !isPresetFile(file)) return;
        const mod = server.moduleGraph.getModuleById(resolvedId);
        if (mod) server.moduleGraph.invalidateModule(mod);
        server.ws.send({ type: "full-reload" });
      };
      server.watcher.on("add", reload);
      server.watcher.on("change", reload);
      server.watcher.on("unlink", reload);
    },
  };
}

export default defineConfig({
  base: process.env.BASE_URL || "/",
  plugins: [
    vue(),
    // Build the Rust ixa model (one level up) to wasm. The `name` matches
    // the wasm-pack output filename (derived from the Cargo crate name with
    // dashes → underscores) so the worker can load `wasm/{name}/{name}.js`.
    cfasimWasm({ model: "..", name: "ixa_basic_transmission" }),
    presetsPlugin(),
  ],
});
