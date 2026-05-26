<script setup lang="ts">
import { computed } from "vue";
import { NumberInput, Button, SelectBox } from "cfasim-ui/components";
import type { SelectOption } from "cfasim-ui/components";
import {
  type InfectionRate,
  empiricalDuration,
  withRateType,
  withPointUpdated,
  withPointAdded,
  withPointRemoved,
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
];

// Two-way bridges for the constant-mode sliders. Each setter emits a
// fresh `Constant` object so the parent's reactivity picks up the
// change. Reads default to 0 when the variant is empirical (the sliders
// aren't mounted then, so this is unobservable).
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

function setRateType(next: string) {
  if (next !== "constant" && next !== "empirical") return;
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
  <template v-else>
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
</style>
