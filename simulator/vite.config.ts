import { defineConfig, type Plugin } from "vite";
import vue from "@vitejs/plugin-vue";
import { cfasimWasm } from "cfasim-ui/wasm/vite";
import { readdirSync, readFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";
import { parse as parseToml } from "smol-toml";

// Reads the rate-library CSV (`id,time,value`) and returns curves grouped
// by id. Returns an empty array if the file is missing. Shared by the
// `virtual:rateLibrary` plugin and the preset-expansion logic below.
function readRateLibrary(): [number, number][][] {
  const file = resolve(
    import.meta.dirname,
    "..",
    "config",
    "rates",
    "library_empirical_rate_fns.csv",
  );
  if (!existsSync(file)) return [];
  const text = readFileSync(file, "utf-8");
  const groups = new Map<string, [number, number][]>();
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.toLowerCase().startsWith("id,")) continue;
    const [idCol, timeCol, valueCol] = line.split(",");
    const t = Number(timeCol);
    const v = Number(valueCol);
    if (!Number.isFinite(t) || !Number.isFinite(v)) continue;
    const arr = groups.get(idCol) ?? [];
    arr.push([t, v]);
    groups.set(idCol, arr);
  }
  const ids = [...groups.keys()].sort((a, b) => Number(a) - Number(b));
  return ids.map((id) => {
    const pts = groups.get(id)!.slice();
    pts.sort((a, b) => a[0] - b[0]);
    return pts;
  });
}

// Reads `../config/parameters/*.{toml,json}` and emits a `virtual:presets`
// module so the frontend can list parameter presets. Each file has
// top-level `name`/`description` plus a `parameters` table. A preset
// declaring `parameters.infectionRate = { type: "library", rates: [] }`
// (or with rates omitted) is expanded at load time to use the bundled
// rate library CSV — keeps presets readable without duplicating the
// data inline.
function presetsPlugin(): Plugin {
  const id = "virtual:presets";
  const resolvedId = "\0" + id;
  const dir = resolve(import.meta.dirname, "..", "config", "parameters");
  const isPresetFile = (f: string) => /\.(toml|json)$/i.test(f);
  function expandLibrary(data: any): any {
    const rate = data?.parameters?.infectionRate;
    if (rate?.type === "library" && (!rate.rates || !rate.rates.length)) {
      rate.rates = readRateLibrary();
    }
    return data;
  }
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
        return { id: f.replace(/\.(toml|json)$/i, ""), ...expandLibrary(data) };
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

// Emits the rate-library CSV (parsed by `readRateLibrary`) as a
// `virtual:rateLibrary` module — the frontend uses this as the bundled
// default library.
function rateLibraryPlugin(): Plugin {
  const id = "virtual:rateLibrary";
  const resolvedId = "\0" + id;
  const file = resolve(
    import.meta.dirname,
    "..",
    "config",
    "rates",
    "library_empirical_rate_fns.csv",
  );
  return {
    name: "ixa-basic-transmission-rate-library",
    resolveId(source) {
      if (source === id) return resolvedId;
    },
    load(loadId) {
      if (loadId !== resolvedId) return;
      return `export default ${JSON.stringify(readRateLibrary())};`;
    },
    configureServer(server) {
      server.watcher.add(file);
      const reload = (changed: string) => {
        if (changed !== file) return;
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
    rateLibraryPlugin(),
  ],
});
