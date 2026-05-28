<script setup lang="ts">
import { computed } from "vue";
import { NumberInput } from "cfasim-ui/components";
import type { PriorBounds } from "../composables/calibration";

const props = defineProps<{
  modelValue: PriorBounds;
}>();
const emit = defineEmits<{
  (e: "update:modelValue", value: PriorBounds): void;
}>();

function patch(p: Partial<PriorBounds>) {
  emit("update:modelValue", { ...props.modelValue, ...p });
}

const r0Lo = computed({
  get: () => props.modelValue.r0Lo,
  set: (v: number) => patch({ r0Lo: v }),
});
const r0Hi = computed({
  get: () => props.modelValue.r0Hi,
  set: (v: number) => patch({ r0Hi: v }),
});
const iiLo = computed({
  get: () => props.modelValue.initialInfectionsLo,
  set: (v: number) => patch({ initialInfectionsLo: Math.max(0, Math.round(v)) }),
});
const iiHi = computed({
  get: () => props.modelValue.initialInfectionsHi,
  set: (v: number) => patch({ initialInfectionsHi: Math.max(0, Math.round(v)) }),
});
</script>

<template>
  <div class="prior-editor">
    <section class="prior-editor__param">
      <h4 class="prior-editor__heading">R<sub>0</sub> (uniform)</h4>
      <div class="prior-editor__bounds">
        <NumberInput v-model="r0Lo" label="Lower" :step="0.1" :min="0" />
        <NumberInput v-model="r0Hi" label="Upper" :step="0.1" :min="0" />
      </div>
    </section>

    <section class="prior-editor__param">
      <h4 class="prior-editor__heading">Initial infections (uniform)</h4>
      <div class="prior-editor__bounds">
        <NumberInput v-model="iiLo" label="Lower" :step="1" :min="0" />
        <NumberInput v-model="iiHi" label="Upper" :step="1" :min="0" />
      </div>
    </section>
  </div>
</template>

<style scoped>
.prior-editor {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}
.prior-editor__heading {
  margin: 0 0 0.25rem;
  font-size: var(--font-size-sm, 0.9rem);
  color: var(--color-text);
}
.prior-editor__bounds {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.5rem;
  margin-top: 0.25rem;
}
</style>
