<script setup lang="ts">
import { NumberInput, Toggle } from "cfasim-ui/components";
import {
  type Facemask,
  type Antiviral,
  DEFAULT_FACEMASK,
  DEFAULT_ANTIVIRAL,
  remainingInfectiousness,
} from "../composables/modifiers";

const props = defineProps<{
  facemask: Facemask | null;
  antiviral: Antiviral | null;
  live?: boolean;
}>();
const emit = defineEmits<{
  (e: "update:facemask", value: Facemask | null): void;
  (e: "update:antiviral", value: Antiviral | null): void;
}>();

// Toggling on seeds a fresh default config; toggling off disables the
// modifier (null → the Rust side reads `None`). Field edits replace the
// whole object — never mutate in place (mirrors the `setParam` rule).
function toggleFacemask(on: boolean) {
  emit("update:facemask", on ? { ...DEFAULT_FACEMASK } : null);
}
function setFacemask(key: keyof Facemask, value: number) {
  if (!props.facemask) return;
  emit("update:facemask", { ...props.facemask, [key]: value });
}

function toggleAntiviral(on: boolean) {
  emit("update:antiviral", on ? { ...DEFAULT_ANTIVIRAL } : null);
}
function setAntiviral(key: keyof Antiviral, value: number) {
  if (!props.antiviral) return;
  emit("update:antiviral", { ...props.antiviral, [key]: value });
}

function pct(fraction: number): string {
  return `${Math.round(remainingInfectiousness(fraction) * 100)}%`;
}
</script>

<template>
  <div class="modifiers-editor">
    <div class="modifiers-header">
      <h3>Transmission modifiers</h3>
      <p class="muted">
        Interventions that reduce an infectious person's intrinsic infectiousness.
        Multiple active modifiers compose multiplicatively.
      </p>
    </div>

    <!-- Facemask -->
    <div class="modifier-card">
      <Toggle :model-value="!!facemask" label="Facemask" hint="Masks donned at a random time during infectiousness."
        @update:model-value="toggleFacemask" />
      <template v-if="facemask">
        <div class="modifier-row">
          <NumberInput :model-value="facemask.coverage" percent label="Coverage" :min="0" :max="1" :live="live"
            @update:model-value="(v: number) => setFacemask('coverage', v)" />
          <NumberInput :model-value="facemask.effectiveness" percent label="Effectiveness" :min="0" :max="1"
            :live="live" @update:model-value="(v: number) => setFacemask('effectiveness', v)" />
        </div>
        <p class="effect">
          Masked people transmit at
          <strong>{{ pct(facemask.effectiveness) }}</strong> the rate
          of baseline.
        </p>
      </template>
    </div>

    <!-- Antiviral -->
    <div class="modifier-card">
      <Toggle :model-value="!!antiviral" label="Antiviral treatment"
        hint="Treatment starts a fixed delay after infection." @update:model-value="toggleAntiviral" />
      <template v-if="antiviral">
        <div class="modifier-row">
          <NumberInput :model-value="antiviral.coverage" percent label="Coverage" :min="0" :max="1" :live="live"
            @update:model-value="(v: number) => setAntiviral('coverage', v)" />
          <NumberInput :model-value="antiviral.efficacy" percent label="Efficacy" :min="0" :max="1" :live="live"
            @update:model-value="(v: number) => setAntiviral('efficacy', v)" />
        </div>
        <div class="modifier-row">
          <NumberInput :model-value="antiviral.delay" label="Treatment delay" :min="0" :step="0.5" :live="live"
            @update:model-value="(v: number) => setAntiviral('delay', v)" />
        </div>
        <p class="effect">
          Treated people transmit at
          <strong>{{ pct(antiviral.efficacy) }}</strong>
          of baseline, starting {{ antiviral.delay }} days after infection.
        </p>
      </template>
    </div>
  </div>
</template>

<style scoped>
.modifiers-editor {
  display: flex;
  flex-direction: column;
  gap: 0.75em;
  margin-block: 0.5em;
}

.modifiers-header {
  display: flex;
  flex-direction: column;
  gap: 0.25em;
}

.modifiers-header h3 {
  margin: 0;
}

.modifiers-header .muted {
  color: var(--cfasim-muted, #666);
  font-size: 0.9em;
  margin: 0;
}

.modifier-card {
  border: 1px solid var(--cfasim-border, #d0d0d0);
  border-radius: 4px;
  padding: 0.75em;
  display: flex;
  flex-direction: column;
  gap: 0.5em;
}

.modifier-row {
  display: flex;
  align-items: end;
  gap: 0.5em;
}

.modifier-row>* {
  flex: 1 1 0;
}

.effect {
  margin: 0;
  font-size: 0.85em;
  color: var(--cfasim-muted, #666);
}
</style>
