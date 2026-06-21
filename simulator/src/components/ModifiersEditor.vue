<script setup lang="ts">
import { computed, shallowRef } from "vue";
import { NumberInput, Toggle, SelectBox } from "cfasim-ui/components";
import type { SelectOption } from "cfasim-ui/components";
import {
  type Facemask,
  type Antiviral,
  type Isolation,
  type Modifiers,
  DEFAULT_FACEMASK,
  DEFAULT_ANTIVIRAL,
  DEFAULT_ISOLATION,
  DEFAULT_ISOLATION_DURATION,
  remainingInfectiousness,
} from "../composables/modifiers";

const props = defineProps<{
  modifiers: Modifiers;
  /** Names of the configured settings — the targets isolation can restrict to. */
  settingNames: string[];
  live?: boolean;
}>();
const emit = defineEmits<{
  (e: "update:modifiers", value: Modifiers): void;
}>();

// All edits replace the whole `modifiers` object (one top-level `setParam`
// key on the parent) — never mutate a nested field in place.
function setModifiers(patch: Partial<Modifiers>) {
  emit("update:modifiers", { ...props.modifiers, ...patch });
}

// When a modifier is toggled off its config is cached here, so toggling it
// back on restores the previous parameters instead of resetting to defaults.
// Component-local (in-memory) — not persisted to the URL/IDB. `shallowRef` +
// spread copies keep the cache a plain (non-reactive) object: a deeply
// reactive `ref` would re-proxy the config, and a Proxy leaking back into the
// `shallowReactive` `params.modifiers` breaks `structuredClone(toRaw(params))`.
const cachedFacemask = shallowRef<Facemask | null>(null);
const cachedAntiviral = shallowRef<Antiviral | null>(null);
const cachedIsolation = shallowRef<Isolation | null>(null);

// Toggling on restores the cached config (or seeds a fresh default the first
// time); toggling off caches the current config then disables the modifier
// (null → the Rust side reads `None`).
function toggleFacemask(on: boolean) {
  if (!on) {
    cachedFacemask.value = props.modifiers.facemask
      ? { ...props.modifiers.facemask }
      : null;
    setModifiers({ facemask: null });
    return;
  }
  setModifiers({ facemask: { ...(cachedFacemask.value ?? DEFAULT_FACEMASK) } });
}
function setFacemask(key: keyof Facemask, value: number) {
  if (!props.modifiers.facemask) return;
  setModifiers({ facemask: { ...props.modifiers.facemask, [key]: value } });
}

function toggleAntiviral(on: boolean) {
  if (!on) {
    cachedAntiviral.value = props.modifiers.antiviral
      ? { ...props.modifiers.antiviral }
      : null;
    setModifiers({ antiviral: null });
    return;
  }
  setModifiers({
    antiviral: { ...(cachedAntiviral.value ?? DEFAULT_ANTIVIRAL) },
  });
}
function setAntiviral(key: keyof Antiviral, value: number) {
  if (!props.modifiers.antiviral) return;
  setModifiers({ antiviral: { ...props.modifiers.antiviral, [key]: value } });
}

// Enabling restores the cached config (or seeds defaults), falling back to the
// first configured setting if the cached restrictTo no longer exists.
function toggleIsolation(on: boolean) {
  if (!on) {
    cachedIsolation.value = props.modifiers.isolation
      ? { ...props.modifiers.isolation }
      : null;
    setModifiers({ isolation: null });
    return;
  }
  const base = cachedIsolation.value ?? DEFAULT_ISOLATION;
  const restrictTo = props.settingNames.includes(base.restrictTo)
    ? base.restrictTo
    : (props.settingNames[0] ?? "");
  setModifiers({ isolation: { ...base, restrictTo } });
}
function setIsolation(patch: Partial<Isolation>) {
  if (!props.modifiers.isolation) return;
  setModifiers({ isolation: { ...props.modifiers.isolation, ...patch } });
}
function toggleIsolationDuration(on: boolean) {
  setIsolation({ duration: on ? DEFAULT_ISOLATION_DURATION : null });
}

const settingOptions = computed<SelectOption[]>(() =>
  props.settingNames.map((n) => ({ value: n, label: n })),
);

function pct(fraction: number): string {
  return `${Math.round(remainingInfectiousness(fraction) * 100)}%`;
}
</script>

<template>
  <div class="modifiers-editor">
    <div class="modifiers-header">
      <h3>Modifiers</h3>
      <p class="muted">
        Interventions that modify infectiousness: by impacting transmission (facemask, antiviral)
        or their contacts (isolation). Active modifiers compose.
      </p>
    </div>

    <!-- Facemask -->
    <div class="modifier-card">
      <Toggle :model-value="!!modifiers.facemask" label="Facemask"
        hint="Masks donned at a random time during infectiousness." @update:model-value="toggleFacemask" />
      <template v-if="modifiers.facemask">
        <div class="modifier-row">
          <NumberInput :model-value="modifiers.facemask.coverage" percent label="Coverage" :min="0" :max="1"
            :live="live" @update:model-value="(v: number) => setFacemask('coverage', v)" />
          <NumberInput :model-value="modifiers.facemask.effectiveness" percent label="Effectiveness" :min="0" :max="1"
            :live="live" @update:model-value="(v: number) => setFacemask('effectiveness', v)" />
        </div>
        <p class="effect">
          Masked people transmit at
          <strong>{{ pct(modifiers.facemask.effectiveness) }}</strong> the rate
          of baseline.
        </p>
      </template>
    </div>

    <!-- Antiviral -->
    <div class="modifier-card">
      <Toggle :model-value="!!modifiers.antiviral" label="Antiviral treatment"
        hint="Treatment starts a fixed delay after infection." @update:model-value="toggleAntiviral" />
      <template v-if="modifiers.antiviral">
        <div class="modifier-row">
          <NumberInput :model-value="modifiers.antiviral.coverage" percent label="Coverage" :min="0" :max="1"
            :live="live" @update:model-value="(v: number) => setAntiviral('coverage', v)" />
          <NumberInput :model-value="modifiers.antiviral.efficacy" percent label="Efficacy" :min="0" :max="1"
            :live="live" @update:model-value="(v: number) => setAntiviral('efficacy', v)" />
        </div>
        <div class="modifier-row">
          <NumberInput :model-value="modifiers.antiviral.delay" label="Treatment delay" :min="0" :step="0.5"
            :live="live" @update:model-value="(v: number) => setAntiviral('delay', v)" />
        </div>
        <p class="effect">
          Treated people transmit at
          <strong>{{ pct(modifiers.antiviral.efficacy) }}</strong>
          of baseline, starting {{ modifiers.antiviral.delay }} days after infection.
        </p>
      </template>
    </div>

    <!-- Isolation (restricts contacts to one setting) -->
    <div class="modifier-card">
      <Toggle :model-value="!!modifiers.isolation" label="Isolation"
        hint="Confine contacts to one setting (e.g. household) after infection." :disabled="settingNames.length === 0"
        @update:model-value="toggleIsolation" />
      <p v-if="settingNames.length === 0" class="muted">
        Add a setting above to enable isolation
      </p>
      <template v-if="modifiers.isolation">
        <div class="modifier-row">
          <NumberInput :model-value="modifiers.isolation.coverage" percent label="Coverage" :min="0" :max="1"
            :live="live" @update:model-value="(v: number) => setIsolation({ coverage: v })" />
          <SelectBox label="Restrict to" :options="settingOptions" :model-value="modifiers.isolation.restrictTo"
            @update:model-value="(v: string) => setIsolation({ restrictTo: v })" />
        </div>
        <div class="modifier-row">
          <NumberInput :model-value="modifiers.isolation.delay" label="Isolation delay" :min="0" :step="0.5"
            :live="live" @update:model-value="(v: number) => setIsolation({ delay: v })" />
        </div>
        <Toggle :model-value="modifiers.isolation.duration !== null" label="Ends after a window"
          @update:model-value="toggleIsolationDuration" />
        <div v-if="modifiers.isolation.duration !== null" class="modifier-row">
          <NumberInput :model-value="modifiers.isolation.duration" label="Isolation duration" :min="0" :step="0.5"
            :live="live" @update:model-value="(v: number) => setIsolation({ duration: v })" />
        </div>
        <p class="effect">
          Isolated people contact only their <strong>{{ modifiers.isolation.restrictTo }}</strong>{{
            modifiers.isolation.duration !== null ? ` for ${modifiers.isolation.duration} days` : ""
          }}, starting {{ modifiers.isolation.delay }} days after infection.
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
