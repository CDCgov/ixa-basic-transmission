<script setup lang="ts">
import { reactive, computed, ref, toRef } from "vue";
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
import { useUrlParams, type ParamCodec } from "cfasim-ui/shared";
import presets from "virtual:presets";
import defaultLibrary from "virtual:rateLibrary";
import RateEditor from "./components/RateEditor.vue";
import {
  type InfectionRate,
  DEFAULT_CONSTANT,
} from "./composables/infectionRate";
import { useSimulationRunner } from "./composables/useSimulationRunner";
import { useChartData } from "./composables/useChartData";

const defaults = {
  infectionRate: { ...DEFAULT_CONSTANT } as InfectionRate,
  population: 10000,
  initialInfections: 5,
  seed: 0,
  maxTime: 100,
  nSimulations: 20,
};
const params = reactive(structuredClone(defaults));

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
    if (r.type === "library") return JSON.stringify({ type: "library" });
    return JSON.stringify(r);
  },
  deserialize: (raw) => {
    const parsed = JSON.parse(raw) as Partial<InfectionRate> & {
      type: string;
    };
    if (parsed.type === "library") {
      return {
        type: "library",
        rates: defaultLibrary.map(
          (c) => c.map((p) => [...p]) as [number, number][],
        ),
      } as InfectionRate;
    }
    return parsed as InfectionRate;
  },
};
const { reset } = useUrlParams(params, defaults, {
  router: useRouter(),
  route: useRoute(),
  codecs: { infectionRate: infectionRateCodec },
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
    const target = preset.parameters[key];
    if (target === undefined) return false;
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
  for (const key of Object.keys(defaults) as (keyof typeof defaults)[]) {
    const v = preset.parameters[key];
    if (v === undefined) continue;
    if (typeof v === "number") {
      if (Number.isFinite(v)) (params as Record<string, unknown>)[key] = v;
    } else if (typeof v === "object" && v !== null) {
      // Clone so subsequent edits don't mutate the preset.
      (params as Record<string, unknown>)[key] = structuredClone(v);
    }
  }
}

function applyParamUpdate(next: ParamEditorValue) {
  // Only assign keys we know about, coercing each one to its default's
  // type so a string "0.5" from a hand-typed JSON value still flows
  // through to the wasm model as a number. Object-shaped fields (like
  // `infectionRate`) are accepted as-is when the JSON shape is valid.
  for (const key of Object.keys(defaults) as (keyof typeof defaults)[]) {
    if (!(key in next)) continue;
    const raw = next[key];
    if (typeof defaults[key] === "number") {
      const n = Number(raw);
      if (Number.isFinite(n)) (params as Record<string, unknown>)[key] = n;
    } else if (typeof raw === "object" && raw !== null) {
      (params as Record<string, unknown>)[key] = structuredClone(raw);
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
);
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
    </template>
  </Teleport>
  <h1>Ixa Basic Transmission</h1>
  <p>
    Stochastic SIR model simulated with
    <a
      href="https://github.com/CDCgov/ixa"
      target="_blank"
      rel="noopener noreferrer"
      >ixa</a
    >. Each newly infectious person schedules their own recovery and their own
    next transmission attempt; targets are picked uniformly from the population.
  </p>
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

<style scoped>
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
