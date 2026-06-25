<script setup lang="ts">
import { shallowReactive, computed, ref, toRef } from "vue";
import { useRouter, useRoute } from "vue-router";
import {
  NumberInput,
  Button,
  Toggle,
  ParamEditor,
  SelectBox,
} from "cfasim-ui/components";
import type { ParamEditorValue, SelectOption } from "cfasim-ui/components";
import { LineChart, DataTable } from "cfasim-ui/charts";
import {
  useUrlParams,
  paramsToQuery,
  jsonCodec,
  type ParamCodec,
} from "cfasim-ui/shared";
import presets from "virtual:presets";
import defaultLibrary from "virtual:rateLibrary";
import RateEditor from "./components/RateEditor.vue";
import RateExplorer from "./components/RateExplorer.vue";
import SettingsEditor from "./components/SettingsEditor.vue";
import ModifiersEditor from "./components/ModifiersEditor.vue";
import settingsLibrary from "virtual:settingsLibrary";
import {
  type InfectionRate,
  DEFAULT_CONSTANT,
  normalizeInfectionRate,
} from "./composables/infectionRate";
import type { SettingType } from "./composables/settings";
import { type Modifiers, DEFAULT_MODIFIERS } from "./composables/modifiers";
import { useSimulationRunner } from "./composables/useSimulationRunner";
import { useChartData } from "./composables/useChartData";

const defaults = {
  infectionRate: { ...DEFAULT_CONSTANT } as InfectionRate,
  population: 10000,
  initialInfections: 5,
  seed: 0,
  maxTime: 100,
  nSimulations: 20,
  settings: [] as SettingType[],
  // Optional transmission modifiers; `null` = disabled (Rust reads `None`).
  modifiers: { ...DEFAULT_MODIFIERS } as Modifiers,
};
type Params = typeof defaults;

const router = useRouter();
const route = useRoute();

// Main-area tab is reflected in the URL path: "/" (or "/simulate") = the run
// charts, "/explore" = the rate-function explainer. The sidebar (incl. the
// rate editor) stays mounted across both, so the explorer reacts live to rate
// edits; switching carries the query string (the model params) along.
const mainTab = computed<"simulate" | "explore">(() =>
  route.params.view === "explore" ? "explore" : "simulate",
);
function setMainTab(tab: "simulate" | "explore") {
  if (tab === mainTab.value) return;
  // `useUrlParams` debounces its URL write (~300ms), so `route.query` can lag
  // the live params. Serialize the current params ourselves and carry them
  // across the path change — otherwise the route watcher would re-hydrate
  // params from a stale query and reset them to defaults.
  router.push({
    path: tab === "explore" ? "/explore" : "/",
    query: paramsToQuery(params, defaults, urlCodecs),
  });
}
const mainTabs: SelectOption[] = [
  { value: "simulate", label: "Simulate" },
  { value: "explore", label: "Explore" },
];

// `shallowReactive` (not `reactive`) so nested values like
// `params.infectionRate.rates` stay as plain arrays. Otherwise Vue
// wraps them in deep reactive Proxies and `useUrlParams`'s internal
// `structuredClone(toRaw(params))` throws DataCloneError on Library
// payloads.
//
// **Rule**: only ever replace whole top-level keys — never mutate a
// nested field in place (`params.infectionRate.scale = 0.05` is a
// silent no-op for reactivity and a footgun for the shape above). The
// allowed write paths are:
//   1. `v-model="params.foo"` in the template (top-level keys only).
//   2. `setParam(key, newValue)` below — one grep target for all
//      non-v-model writes.
const params = shallowReactive(structuredClone(defaults));
function setParam<K extends keyof Params>(key: K, value: Params[K]): void {
  params[key] = value;
}

// `infectionRate` is a tagged union — useUrlParams needs a codec to
// round-trip variants whose shape differs from the default. We JSON-
// encode Constant and Empirical in full, but for Library we elide the
// (potentially huge) `rates` payload and rehydrate it from the bundled
// `virtual:rateLibrary` on the way back in. A user-uploaded CSV is
// session-local and doesn't round-trip — which is the expected
// behavior for an upload.
const infectionRateCodec: ParamCodec = {
  serialize: (value) => {
    const r = value as InfectionRate;
    // For Library, drop the (potentially huge) `rates` payload but
    // keep `scale` so a user's calibration round-trips through the URL.
    if (r.type === "library") {
      return JSON.stringify({ type: "library", scale: r.scale });
    }
    return JSON.stringify(r);
  },
  deserialize: (raw) => {
    const parsed = JSON.parse(raw) as Partial<InfectionRate> & {
      type: string;
      scale?: number;
    };
    if (parsed.type === "library") {
      return normalizeInfectionRate({
        type: "library",
        rates: defaultLibrary.map(
          (c) => c.map((p) => [...p]) as [number, number][],
        ),
        scale: parsed.scale ?? 1.0,
      });
    }
    return normalizeInfectionRate(parsed as InfectionRate);
  },
};
// Settings is a Vec<SettingType>. Without a codec, useUrlParams would
// flatten it to dotted keys per array entry — verbose and brittle as
// users add/remove setting types. Use a hybrid encoding instead:
//
//   * If `value` exactly matches a `virtual:settingsLibrary` preset,
//     serialize as the preset id (e.g. `households_us`). Keeps the
//     URL compact and human-readable.
//   * Otherwise JSON-encode the array with probabilities rounded to
//     2 decimals. UI inputs use `step=0.01`, so values that came in via
//     the editor are already 2 dp and round-trip cleanly; for
//     hand-edited values that were finer-grained, the rounded sum can
//     drift outside `SettingType::validate`'s 1e-6 tolerance — use the
//     "Normalize" button to restore.
//
// Deserialize is the reverse: a leading `[` means JSON, anything else
// is treated as a preset id.
function roundProb(p: number): number {
  return Math.round(p * 100) / 100;
}
function findLibraryId(settings: SettingType[]): string | undefined {
  const target = JSON.stringify(settings);
  return settingsLibrary.find((p) => JSON.stringify(p.settings) === target)?.id;
}
const settingsCodec: ParamCodec = {
  serialize: (value) => {
    const settings = value as SettingType[];
    const presetId = findLibraryId(settings);
    if (presetId) return presetId;
    const rounded = settings.map((s) => ({
      ...s,
      alpha: roundProb(s.alpha),
      proportion: roundProb(s.proportion),
      sizes: s.sizes.map(([n, p]) => [n, roundProb(p)] as [number, number]),
    }));
    return JSON.stringify(rounded);
  },
  deserialize: (raw) => {
    if (raw.startsWith("[")) return JSON.parse(raw) as SettingType[];
    const entry = settingsLibrary.find((p) => p.id === raw);
    if (!entry) return [];
    // Deep-clone so user edits don't mutate the bundled preset.
    return entry.settings.map((s) => ({
      ...s,
      sizes: s.sizes.map((p) => [...p] as [number, number]),
    }));
  },
};
// Shared with `setMainTab`, which serializes params to carry them across a
// tab path change (see there). `modifiers` uses `jsonCodec`: the default
// `null` serializes to "null" and matches, so it's omitted from the URL until
// a modifier is enabled.
const urlCodecs = {
  infectionRate: infectionRateCodec,
  settings: settingsCodec,
  modifiers: jsonCodec,
};
const { reset } = useUrlParams(params, defaults, {
  router,
  route,
  codecs: urlCodecs,
});

// When `useEditor` is on, the sidebar shows a JSON/TOML/YAML editor
// instead of the per-field NumberInput form. Both views share the same
// underlying `params` so users can flip back and forth.
const useEditor = ref(false);

// Presets are loaded at build time from `config/*.toml` via the
// `virtual:presets` Vite plugin. Selection is derived from `params`: if
// they match a preset exactly, that preset is shown as selected;
// otherwise the picker shows "Custom".
const presetOptions: SelectOption[] = presets.map((p) => ({
  value: p.id,
  label: p.name,
}));

function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || !a || !b) return false;
  const aKeys = Object.keys(a as Record<string, unknown>);
  const bKeys = Object.keys(b as Record<string, unknown>);
  if (aKeys.length !== bKeys.length) return false;
  for (const k of aKeys) {
    if (
      !deepEqual(
        (a as Record<string, unknown>)[k],
        (b as Record<string, unknown>)[k],
      )
    )
      return false;
  }
  return true;
}

function paramsMatchPreset(preset: (typeof presets)[number]): boolean {
  for (const key of Object.keys(defaults) as (keyof typeof defaults)[]) {
    // A preset that omits a key is treated as inheriting the default —
    // e.g. older presets predate the `settings` field but should still
    // match the defaults when nothing else differs.
    const target = preset.parameters[key] ?? defaults[key];
    if (!deepEqual((params as Record<string, unknown>)[key], target))
      return false;
  }
  return true;
}

const selectedPresetId = computed<string>(
  () => presets.find(paramsMatchPreset)?.id ?? "",
);

const selectedPresetDescription = computed<string | undefined>(
  () => presets.find((p) => p.id === selectedPresetId.value)?.description,
);

function applyPreset(id: string) {
  const preset = presets.find((p) => p.id === id);
  if (!preset) return;
  for (const key of Object.keys(defaults) as (keyof Params)[]) {
    // Omitted preset keys reset to defaults — mirrors `paramsMatchPreset`
    // so a preset that doesn't declare every field stays selected after
    // it's applied (otherwise stale values from the previous preset
    // could leak in and the picker would jump to "Custom").
    const v = preset.parameters[key] ?? defaults[key];
    if (v === null) {
      // Nullable params (facemask/antiviral): a preset omitting them, or
      // setting them null, disables the modifier — set explicitly so a
      // stale value from the previous preset can't leak through.
      setParam(key, null as Params[typeof key]);
    } else if (typeof v === "number") {
      if (Number.isFinite(v)) setParam(key, v as Params[typeof key]);
    } else if (typeof v === "object") {
      // Clone so subsequent edits don't mutate the preset / defaults.
      // Normalize infectionRate so older presets without `scale` get
      // scale=1.0.
      const cloned = structuredClone(v);
      const next =
        key === "infectionRate"
          ? normalizeInfectionRate(cloned as InfectionRate)
          : cloned;
      setParam(key, next as Params[typeof key]);
    }
  }
}

function applyParamUpdate(next: ParamEditorValue) {
  // Only assign keys we know about, coercing each one to its default's
  // type so a string "0.5" from a hand-typed JSON value still flows
  // through to the wasm model as a number. Object-shaped fields (like
  // `infectionRate`) are accepted as-is when the JSON shape is valid.
  for (const key of Object.keys(defaults) as (keyof Params)[]) {
    if (!(key in next)) continue;
    const raw = next[key];
    if (raw === null) {
      // Only nullable params (default `null`) accept null = disabled;
      // ignore a stray null on a non-nullable field.
      if (defaults[key] === null) setParam(key, null as Params[typeof key]);
    } else if (typeof defaults[key] === "number") {
      const n = Number(raw);
      if (Number.isFinite(n)) setParam(key, n as Params[typeof key]);
    } else if (typeof raw === "object") {
      const cloned = structuredClone(raw);
      const nextVal =
        key === "infectionRate"
          ? normalizeInfectionRate(cloned as InfectionRate)
          : cloned;
      setParam(key, nextVal as Params[typeof key]);
    }
  }
}

// Workload threshold above which slider inputs commit on blur instead of
// firing on every drag tick (consumed by NumberInput's `:live`).
const LARGE_WORKLOAD = 20_000;
const live = computed(
  () => params.nSimulations * params.population <= LARGE_WORKLOAD,
);

const { outputs, error, loading, statusMessage } = useSimulationRunner(params, {
  wasmName: "ixa_basic_transmission",
});
const { charts, summary, fmtCount } = useChartData(
  outputs,
  toRef(params, "infectionRate"),
  toRef(params, "settings"),
);

// Bigger axis text for the main result charts. Passed as props (not CSS) so the
// chart reserves the correct gutter for the larger tick/axis labels.
const axisTextStyle = { fontSize: 13 };
</script>

<template>
  <Teleport to="#model-sidebar">
    <div class="sidebar-header">
      <h2>Parameters</h2>
      <SelectBox
        v-if="presetOptions.length"
        label="Preset"
        :options="presetOptions"
        :model-value="selectedPresetId"
        placeholder="Custom"
        @update:model-value="applyPreset"
      />
      <p v-if="selectedPresetDescription" class="preset-description">
        {{ selectedPresetDescription }}
      </p>
      <div class="sidebar-controls">
        <Toggle v-model="useEditor" label="Edit as code" />
        <Button variant="secondary" @click="reset">Reset</Button>
      </div>
    </div>
    <!-- Snapshot params so the editor sees a fresh object reference each
         re-render and its internal "external update" watcher fires when
         the parent applies a save (or a Reset). -->
    <ParamEditor
      v-if="useEditor"
      :value="{ ...params }"
      format="json"
      @apply="applyParamUpdate"
    />
    <template v-else>
      <RateEditor v-model="params.infectionRate" :live="live" />
      <NumberInput
        v-model="params.population"
        label="Population"
        :min="10"
        :max="100000"
      />
      <NumberInput
        v-model="params.initialInfections"
        label="Initial infections"
        :min="1"
      />
      <NumberInput v-model="params.seed" label="Seed" :min="0" />
      <NumberInput v-model="params.maxTime" label="Max time" :min="1" />
      <NumberInput
        v-model="params.nSimulations"
        label="Number of simulations"
        :min="1"
        :max="100"
      />
      <SettingsEditor v-model="params.settings" :live="live" />
      <ModifiersEditor
        :modifiers="params.modifiers"
        :setting-names="params.settings.map((s) => s.name)"
        :live="live"
        @update:modifiers="(v: Modifiers) => setParam('modifiers', v)"
      />
    </template>
  </Teleport>
  <h1>Ixa Basic Transmission</h1>
  <p>
    Stochastic transmission model simulated with
    <a
      href="https://github.com/CDCgov/ixa"
      target="_blank"
      rel="noopener noreferrer"
      >Ixa</a
    >. Each newly infectious person schedules their own recovery and their own
    next transmission attempt; contacts are picked from a setting in their
    itinerary, or uniformly from the population when no settings are configured.
  </p>
  <div class="main-tabs" role="tablist">
    <button
      v-for="tab in mainTabs"
      :key="tab.value"
      type="button"
      role="tab"
      class="main-tab"
      :class="{ 'main-tab-active': mainTab === tab.value }"
      :aria-selected="mainTab === tab.value"
      @click="setMainTab(tab.value as 'simulate' | 'explore')"
    >
      {{ tab.label }}
    </button>
  </div>
  <template v-if="mainTab === 'simulate'">
    <p v-if="error" class="error">{{ error }}</p>
    <p v-if="statusMessage" class="status">{{ statusMessage }}</p>
    <template v-for="chart in charts" :key="chart.yLabel">
      <LineChart
        v-if="chart.series.length"
        :series="chart.series"
        :annotations="chart.annotations"
        :chart-padding="chart.padding"
        :height="chart.height"
        x-label="Time"
        :y-label="chart.yLabel"
        :tick-label-style="axisTextStyle"
        :axis-label-style="axisTextStyle"
        :legend-style="axisTextStyle"
        tooltip-trigger="hover"
      >
        <template #tooltip="{ xLabel, values }">
          <div class="chart-tooltip">
            <div v-if="xLabel" class="chart-tooltip-label">{{ xLabel }}</div>
            <div class="chart-tooltip-row">
              <span class="chart-tooltip-median">Median</span>
              <span>{{ fmtCount(values[values.length - 1]?.value) }}</span>
            </div>
          </div>
        </template>
      </LineChart>
    </template>
    <DataTable
      v-if="summary"
      :data="summary"
      :column-config="{
        metric: { label: 'Metric' },
        value: { label: 'Value', align: 'right' },
      }"
      :menu="false"
    />
  </template>
  <RateExplorer v-else :model-value="params.infectionRate" />
</template>

<style scoped>
.main-tabs {
  display: flex;
  gap: 0.25rem;
  border-bottom: 1px solid var(--color-border);
  margin: 0.5em 0 1em;
}
.main-tab {
  appearance: none;
  background: none;
  border: none;
  border-bottom: 2px solid transparent;
  margin-bottom: -1px;
  padding: 0.5em 0.75em;
  font-size: var(--font-size-sm, 0.875rem);
  color: var(--color-text-secondary);
  cursor: pointer;
}
.main-tab:hover {
  color: var(--color-text);
}
.main-tab-active {
  color: var(--color-text);
  border-bottom-color: var(--color-accent, #2563eb);
  font-weight: 600;
}
.sidebar-header {
  display: flex;
  flex-direction: column;
  gap: 0.5em;
}
.sidebar-header h2 {
  margin: 0;
}
.sidebar-controls {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5em;
}
.preset-description {
  margin: 0;
  font-size: var(--font-size-sm, 0.875rem);
  color: var(--cfa-color-text-muted, #666);
}
.error {
  color: red;
}
.status {
  color: var(--cfa-color-text-muted, #666);
}
.chart-tooltip {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 9rem;
}
.chart-tooltip-label {
  font-weight: 500;
}
.chart-tooltip-row {
  display: flex;
  justify-content: space-between;
  gap: 12px;
}
.chart-tooltip-median {
  color: #f87171;
}
</style>
