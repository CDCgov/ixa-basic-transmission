<script setup lang="ts">
import { computed, ref } from "vue";
import { NumberInput, Button, SelectBox } from "cfasim-ui/components";
import type { SelectOption } from "cfasim-ui/components";
import { LineChart } from "cfasim-ui/charts";
import defaultLibrary from "virtual:rateLibrary";
import {
  type InfectionRate,
  empiricalDuration,
  withRateType,
  withPointUpdated,
  withPointAdded,
  withPointRemoved,
  parseRateLibraryCsv,
} from "../composables/infectionRate";

const props = defineProps<{
  modelValue: InfectionRate;
  live?: boolean;
}>();
const emit = defineEmits<{
  (e: "update:modelValue", value: InfectionRate): void;
}>();

const rateTypeOptions: SelectOption[] = [
  { value: "constant", label: "Constant" },
  { value: "empirical", label: "Time-varying" },
  { value: "library", label: "Library" },
];

// Two-way bridges for the constant-mode sliders. Each setter emits a
// fresh `Constant` object so the parent's reactivity picks up the
// change. Reads default to 0 when the variant doesn't match (the
// sliders aren't mounted then, so this is unobservable).
const constantValue = computed<number>({
  get: () =>
    props.modelValue.type === "constant" ? props.modelValue.value : 0,
  set: (v) => {
    if (props.modelValue.type !== "constant") return;
    emit("update:modelValue", {
      type: "constant",
      value: v,
      duration: props.modelValue.duration,
    });
  },
});

const constantDuration = computed<number>({
  get: () =>
    props.modelValue.type === "constant" ? props.modelValue.duration : 0,
  set: (d) => {
    if (props.modelValue.type !== "constant") return;
    emit("update:modelValue", {
      type: "constant",
      value: props.modelValue.value,
      duration: d,
    });
  },
});

const empiricalPoints = computed<[number, number][]>(() =>
  props.modelValue.type === "empirical" ? props.modelValue.points : [],
);

const empiricalRecoveryAt = computed(() => empiricalDuration(props.modelValue));

const libraryRates = computed<[number, number][][]>(() =>
  props.modelValue.type === "library" ? props.modelValue.rates : [],
);
const libraryCount = computed(() => libraryRates.value.length);

// Pagination: 2×2 grid of mini charts.
const PAGE_SIZE = 4;
const page = ref(0);
const pageCount = computed(() =>
  Math.max(1, Math.ceil(libraryCount.value / PAGE_SIZE)),
);
const pageStart = computed(() => page.value * PAGE_SIZE);
const pagedCurves = computed(() =>
  libraryRates.value.slice(pageStart.value, pageStart.value + PAGE_SIZE),
);

function clampPage() {
  if (page.value >= pageCount.value) page.value = pageCount.value - 1;
  if (page.value < 0) page.value = 0;
}

function prevPage() {
  if (page.value > 0) {
    page.value -= 1;
  }
}
function nextPage() {
  if (page.value < pageCount.value - 1) {
    page.value += 1;
  }
}

function setRateType(next: string) {
  if (next !== "constant" && next !== "empirical" && next !== "library") {
    return;
  }
  if (next === "library") {
    page.value = 0;
    emit("update:modelValue", withRateType(props.modelValue, "library", defaultLibrary));
    return;
  }
  emit("update:modelValue", withRateType(props.modelValue, next));
}

function updatePoint(i: number, axis: 0 | 1, value: number) {
  emit("update:modelValue", withPointUpdated(props.modelValue, i, axis, value));
}

function addPoint() {
  emit("update:modelValue", withPointAdded(props.modelValue));
}

function removePoint(i: number) {
  emit("update:modelValue", withPointRemoved(props.modelValue, i));
}

// CSV upload — replaces the library payload for the session. The
// bundled `defaultLibrary` stays available via the rate-type re-select.
const fileInput = ref<HTMLInputElement | null>(null);
const uploadError = ref<string | null>(null);

function triggerUpload() {
  uploadError.value = null;
  fileInput.value?.click();
}

async function onFile(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  if (!file) return;
  uploadError.value = null;
  try {
    const text = await file.text();
    const rates = parseRateLibraryCsv(text);
    if (!rates.length) {
      throw new Error("library is empty after parsing");
    }
    page.value = 0;
    emit("update:modelValue", { type: "library", rates });
  } catch (e) {
    uploadError.value = (e as Error).message || "failed to parse CSV";
  } finally {
    input.value = "";
  }
}

function restoreDefaultLibrary() {
  page.value = 0;
  uploadError.value = null;
  emit("update:modelValue", {
    type: "library",
    rates: defaultLibrary.map((c) =>
      c.map((p) => [...p] as [number, number]),
    ),
  });
}

// Single-series preview tracing λ(τ): a flat segment for Constant,
// linear interpolation through anchor points for Empirical. For
// Library, the editor renders a grid below — this preview is suppressed.
const previewSeries = computed(() => {
  const rate = props.modelValue;
  if (rate.type === "constant") {
    return [
      {
        x: [0, rate.duration],
        data: [rate.value, rate.value],
        color: "#2563eb",
        strokeWidth: 2,
      },
    ];
  }
  if (rate.type === "empirical") {
    return [
      {
        x: rate.points.map((p) => p[0]),
        data: rate.points.map((p) => p[1]),
        color: "#2563eb",
        strokeWidth: 2,
        dots: true,
      },
    ];
  }
  return [];
});

function curveSeries(curve: [number, number][]) {
  return [
    {
      x: curve.map((p) => p[0]),
      data: curve.map((p) => p[1]),
      color: "#2563eb",
      strokeWidth: 1.5,
      dots: false,
    },
  ];
}

// Y-axis max shared across the grid so the curves are visually
// comparable page-to-page.
const libraryYMax = computed(() => {
  let m = 0;
  for (const curve of libraryRates.value) {
    for (const [, v] of curve) {
      if (Number.isFinite(v) && v > m) m = v;
    }
  }
  // Padding so the curve isn't flush against the chart top.
  return m > 0 ? m * 1.1 : 1;
});

// If the library shrinks after a CSV swap, snap back to a valid page.
function watchSize() {
  clampPage();
}
</script>

<template>
  <SelectBox
    label="Infectiousness"
    :options="rateTypeOptions"
    :model-value="modelValue.type"
    @update:model-value="setRateType"
  />
  <template v-if="modelValue.type === 'constant'">
    <NumberInput
      v-model="constantValue"
      label="Infection rate"
      slider
      :live="live"
      :min="0.05"
      :max="2"
      :step="0.05"
    />
    <NumberInput
      v-model="constantDuration"
      label="Infectious period"
      slider
      :live="live"
      :min="1"
      :max="14"
      :step="0.5"
    />
  </template>
  <template v-else-if="modelValue.type === 'empirical'">
    <p class="schedule-hint">
      Time-varying curve, recovery at τ = {{ empiricalRecoveryAt }}.
    </p>
    <div class="points-editor">
      <div class="points-header">
        <span>τ (time since infected)</span>
        <span>rate</span>
        <span></span>
      </div>
      <div
        v-for="(point, i) in empiricalPoints"
        :key="`${point[0]}-${point[1]}-${i}`"
        class="points-row"
      >
        <NumberInput
          :model-value="point[0]"
          :min="0"
          :step="0.5"
          @update:model-value="(v: number) => updatePoint(i, 0, v)"
        />
        <NumberInput
          :model-value="point[1]"
          :min="0"
          :step="0.1"
          @update:model-value="(v: number) => updatePoint(i, 1, v)"
        />
        <Button
          variant="secondary"
          :disabled="empiricalPoints.length <= 2"
          @click="removePoint(i)"
          >×</Button
        >
      </div>
      <Button variant="secondary" @click="addPoint">Add point</Button>
    </div>
  </template>
  <template v-else>
    <p class="schedule-hint">
      Library of {{ libraryCount }} per-person curves. Each person draws
      one uniformly at setup.
    </p>
    <div class="library-actions">
      <Button variant="secondary" @click="triggerUpload">Upload CSV</Button>
      <Button variant="secondary" @click="restoreDefaultLibrary"
        >Restore default</Button
      >
      <input
        ref="fileInput"
        type="file"
        accept=".csv,text/csv"
        class="library-file-input"
        @change="onFile"
      />
    </div>
    <p v-if="uploadError" class="library-error">{{ uploadError }}</p>
    <div v-if="libraryCount" class="library-grid" @vue:mounted="watchSize">
      <div
        v-for="(curve, i) in pagedCurves"
        :key="pageStart + i"
        class="library-cell"
      >
        <div class="library-cell-label">#{{ pageStart + i + 1 }}</div>
        <LineChart
          :series="curveSeries(curve)"
          :height="100"
          :y-min="0"
          :y-max="libraryYMax"
          :menu="false"
          :axis-label-style="{ fontSize: 9 }"
          :tick-label-style="{ fontSize: 9 }"
        />
      </div>
    </div>
    <div v-if="pageCount > 1" class="library-pager">
      <Button
        variant="secondary"
        :disabled="page === 0"
        @click="prevPage"
        >‹ Prev</Button
      >
      <span class="library-page-info"
        >Page {{ page + 1 }} / {{ pageCount }}</span
      >
      <Button
        variant="secondary"
        :disabled="page >= pageCount - 1"
        @click="nextPage"
        >Next ›</Button
      >
    </div>
  </template>
  <div v-if="modelValue.type !== 'library'" class="rate-preview">
    <LineChart
      :series="previewSeries"
      :height="120"
      :y-min="0"
      :menu="false"
      x-label="τ (days since infected)"
      y-label="rate"
      :axis-label-style="{ fontSize: 10 }"
      :tick-label-style="{ fontSize: 10 }"
    />
  </div>
</template>

<style scoped>
.schedule-hint {
  margin: 0;
  font-size: var(--font-size-sm, 0.875rem);
  color: var(--cfa-color-text-muted, #666);
}
.points-editor {
  display: flex;
  flex-direction: column;
  gap: 0.4em;
}
.points-header {
  display: grid;
  grid-template-columns: 1fr 1fr 2em;
  gap: 0.4em;
  font-size: var(--font-size-xs, 0.75rem);
  color: var(--cfa-color-text-muted, #666);
}
.points-row {
  display: grid;
  grid-template-columns: 1fr 1fr 2em;
  gap: 0.4em;
  align-items: center;
}
.rate-preview {
  margin-top: 0.5em;
}
.library-actions {
  display: flex;
  gap: 0.4em;
  flex-wrap: wrap;
}
.library-file-input {
  display: none;
}
.library-error {
  margin: 0;
  font-size: var(--font-size-sm, 0.875rem);
  color: #dc2626;
}
.library-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.4em;
  margin-top: 0.5em;
}
.library-cell {
  display: flex;
  flex-direction: column;
  gap: 0.2em;
  border: 1px solid var(--cfa-color-border, #e5e5e5);
  border-radius: 4px;
  padding: 4px;
}
.library-cell-label {
  font-size: var(--font-size-xs, 0.75rem);
  color: var(--cfa-color-text-muted, #666);
}
.library-pager {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: 0.5em;
  gap: 0.5em;
}
.library-page-info {
  font-size: var(--font-size-sm, 0.875rem);
  color: var(--cfa-color-text-muted, #666);
}
</style>
