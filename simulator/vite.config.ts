import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { cfasimWasm } from "@cfasim-ui/wasm/vite";

export default defineConfig({
  base: process.env.BASE_URL || "/",
  plugins: [
    vue(),
    // Build the Rust ixa model (one level up) to wasm. The `name` matches
    // the wasm-pack output filename (derived from the Cargo crate name with
    // dashes → underscores) so the worker can load `wasm/{name}/{name}.js`.
    cfasimWasm({ model: "..", name: "ixa_basic_transmission" }),
  ],
});
