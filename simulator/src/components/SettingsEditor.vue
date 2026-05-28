<script setup lang="ts">
import { computed } from "vue";
import { NumberInput, TextInput, Button, SelectBox } from "cfasim-ui/components";
import type { SelectOption } from "cfasim-ui/components";
import { BarChart } from "cfasim-ui/charts";
import settingsLibrary from "virtual:settingsLibrary";
import {
  type SettingType,
  DEFAULT_SETTING_TYPE,
  withSizeAdded,
  withSizeRemoved,
  withSizeUpdated,
  withSizesNormalized,
  sizesProbabilitySum,
  expectedSettingMultiplier,
} from "../composables/settings";

const props = defineProps<{
  modelValue: SettingType[];
  live?: boolean;
}>();
const emit = defineEmits<{
  (e: "update:modelValue", value: SettingType[]): void;
}>();

const settings = computed(() => props.modelValue);

function commit(next: SettingType[]) {
  emit("update:modelValue", next);
}

function updateAt(i: number, next: SettingType) {
  commit(settings.value.map((s, idx) => (idx === i ? next : s)));
}

function addSetting() {
  const seed: SettingType = {
    ...DEFAULT_SETTING_TYPE,
    name:
      settings.value.length === 0
        ? DEFAULT_SETTING_TYPE.name
        : `setting ${settings.value.length + 1}`,
    sizes: DEFAULT_SETTING_TYPE.sizes.map((p) => [...p] as [number, number]),
  };
  commit([...settings.value, seed]);
}

function removeSetting(i: number) {
  commit(settings.value.filter((_, idx) => idx !== i));
}

function setName(i: number, name: string) {
  updateAt(i, { ...settings.value[i], name });
}

function setAlpha(i: number, alpha: number) {
  updateAt(i, { ...settings.value[i], alpha });
}

function setProportion(i: number, proportion: number) {
  updateAt(i, { ...settings.value[i], proportion });
}

function addSize(i: number) {
  updateAt(i, withSizeAdded(settings.value[i]));
}

function removeSize(i: number, j: number) {
  updateAt(i, withSizeRemoved(settings.value[i], j));
}

function updateSize(i: number, j: number, axis: 0 | 1, value: number) {
  updateAt(i, withSizeUpdated(settings.value[i], j, axis, value));
}

function normalize(i: number) {
  updateAt(i, withSizesNormalized(settings.value[i]));
}

const proportionTotal = computed(() =>
  settings.value.reduce(
    (acc, s) => acc + (Number.isFinite(s.proportion) ? s.proportion : 0),
    0,
  ),
);

const libraryOptions = computed<SelectOption[]>(() =>
  settingsLibrary.map((p) => ({ value: p.id, label: p.name })),
);

// `selectedLibraryId` reflects which library entry (if any) the current
// `settings` exactly match. Lets the SelectBox show "households_us" when
// the user has the US default in place, but switch to nothing once they
// edit a value.
const selectedLibraryId = computed<string>(() => {
  const match = settingsLibrary.find(
    (p) => JSON.stringify(p.settings) === JSON.stringify(settings.value),
  );
  return match?.id ?? "";
});

function applyLibrary(id: string) {
  const entry = settingsLibrary.find((p) => p.id === id);
  if (!entry) return;
  // Deep-clone so user edits to `settings` don't mutate the bundled preset.
  commit(
    entry.settings.map((s) => ({
      ...s,
      sizes: s.sizes.map((p) => [...p] as [number, number]),
    })),
  );
}

function sizeChartData(s: SettingType) {
  const filtered = s.sizes.filter(
    ([n, p]) => Number.isFinite(n) && Number.isFinite(p),
  );
  return {
    categories: filtered.map(([n]) => String(n)),
    data: filtered.map(([, p]) => Math.max(p, 0)),
  };
}
</script>

<template>
  <div class="settings-editor">
    <div class="settings-header">
      <h3>Settings</h3>
      <SelectBox
        v-if="libraryOptions.length"
        label="Library"
        :options="libraryOptions"
        :model-value="selectedLibraryId"
        placeholder="Custom"
        @update:model-value="applyLibrary"
      />
      <p v-if="settings.length === 0" class="muted">
        Global random mixing. Add a setting to use density-scaled contact
        groups.
      </p>
      <p
        v-else-if="Math.abs(proportionTotal - 1) > 0.01"
        class="warn"
      >
        Itinerary proportions sum to {{ proportionTotal.toFixed(3) }} (the
        model rate-weights them internally, but a sum of 1 is conventional).
      </p>
    </div>

    <div
      v-for="(s, i) in settings"
      :key="i"
      class="setting-card"
    >
      <button
        type="button"
        class="icon-button card-remove"
        aria-label="Remove setting"
        @click="removeSetting(i)"
      >
        ×
      </button>
      <div class="setting-row">
        <TextInput
          :model-value="s.name"
          label="Name"
          @update:model-value="(v: string) => setName(i, v)"
        />
      </div>
      <div class="setting-row">
        <NumberInput
          :model-value="s.alpha"
          label="Alpha (density exponent)"
          :min="0"
          :max="1"
          :step="0.05"
          :live="live"
          @update:model-value="(v: number) => setAlpha(i, v)"
        />
        <NumberInput
          :model-value="s.proportion"
          label="Itinerary proportion"
          :min="0"
          :step="0.05"
          :live="live"
          @update:model-value="(v: number) => setProportion(i, v)"
        />
      </div>
      <div class="setting-row size-summary">
        <span>
          Expected M = E[(n − 1)<sup>α</sup>] ≈
          {{ expectedSettingMultiplier(s).toFixed(3) }}
        </span>
        <span
          :class="{
            warn: Math.abs(sizesProbabilitySum(s) - 1) > 0.005,
          }"
        >
          Σ P(size) = {{ sizesProbabilitySum(s).toFixed(3) }}
        </span>
        <Button variant="secondary" @click="normalize(i)">
          Normalize
        </Button>
      </div>

      <BarChart
        v-if="s.sizes.length > 0"
        v-bind="sizeChartData(s)"
        :height="120"
        x-label="Setting size"
        y-label="Probability"
      />

      <div class="size-table">
        <div class="size-table-header">
          <span>Size</span>
          <span>Probability</span>
          <span></span>
        </div>
        <div
          v-for="(p, j) in s.sizes"
          :key="j"
          class="size-table-row"
        >
          <NumberInput
            :model-value="p[0]"
            :min="1"
            :step="1"
            :live="live"
            label=""
            @update:model-value="(v: number) => updateSize(i, j, 0, v)"
          />
          <NumberInput
            :model-value="p[1]"
            :min="0"
            :step="0.01"
            :live="live"
            label=""
            @update:model-value="(v: number) => updateSize(i, j, 1, v)"
          />
          <button
            type="button"
            class="icon-button"
            aria-label="Remove size"
            :disabled="s.sizes.length <= 1"
            @click="removeSize(i, j)"
          >
            ×
          </button>
        </div>
        <Button variant="secondary" @click="addSize(i)">+ Add size</Button>
      </div>
    </div>

    <Button @click="addSetting">+ Add setting</Button>
  </div>
</template>

<style scoped>
.settings-editor {
  display: flex;
  flex-direction: column;
  gap: 0.75em;
  margin-block: 0.5em;
}

.settings-header h3 {
  margin: 0 0 0.25em 0;
}

.settings-header .muted {
  color: var(--cfasim-muted, #666);
  font-size: 0.9em;
  margin: 0;
}

.settings-header .warn,
.size-summary .warn {
  color: var(--cfasim-warn, #b54708);
}

.setting-card {
  position: relative;
  border: 1px solid var(--cfasim-border, #d0d0d0);
  border-radius: 4px;
  padding: 0.75em;
  display: flex;
  flex-direction: column;
  gap: 0.5em;
}

.icon-button {
  width: 1.5em;
  height: 1.5em;
  padding: 0;
  border: 1px solid var(--cfasim-border, #d0d0d0);
  background: transparent;
  border-radius: 3px;
  font-size: 1.1em;
  line-height: 1;
  cursor: pointer;
  color: var(--cfasim-muted, #666);
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.icon-button:hover:not(:disabled) {
  color: var(--cfasim-warn, #b54708);
  border-color: var(--cfasim-warn, #b54708);
}

.icon-button:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.card-remove {
  position: absolute;
  top: 0.5em;
  right: 0.5em;
}

.setting-row {
  display: flex;
  align-items: end;
  gap: 0.5em;
}

.setting-row > * {
  flex: 1 1 0;
}

.size-summary {
  font-size: 0.85em;
  align-items: center;
  flex-wrap: wrap;
}

.size-table {
  display: flex;
  flex-direction: column;
  gap: 0.25em;
}

.size-table-header {
  display: grid;
  grid-template-columns: 1fr 1fr auto;
  gap: 0.5em;
  font-size: 0.8em;
  font-weight: 600;
  color: var(--cfasim-muted, #666);
}

.size-table-row {
  display: grid;
  grid-template-columns: 1fr 1fr auto;
  gap: 0.5em;
  align-items: end;
}
</style>
