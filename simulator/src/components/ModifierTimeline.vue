<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";

// SVG timeline over [0, duration] for one method: activation marker, solid
// event dots, hollow discarded-forecast dots, or a sim round's jittered dots.
const props = defineProps<{
  duration: number;
  activatesAt: number;
  events: number[];
  discarded: number[];
  /** Latest simulate round's event times; `null` in draw mode. */
  simTimes: number[] | null;
  recovered: boolean;
  color: string;
  discardedLabel: string;
}>();

const HEIGHT = 56;
const PAD_R = 28;
const AXIS_Y = 24;

const el = ref<HTMLElement | null>(null);
const width = ref(560);
let ro: ResizeObserver | null = null;
onMounted(() => {
  ro = new ResizeObserver((entries) => {
    const w = entries[0]?.contentRect.width;
    if (w) width.value = w;
  });
  if (el.value) ro.observe(el.value);
});
onBeforeUnmount(() => ro?.disconnect());

function tx(t: number): number {
  const d = props.duration || 1;
  const inner = Math.max(1, width.value - PAD_R);
  return Math.min(1, Math.max(0, t / d)) * inner;
}

const simMode = computed(() => props.simTimes !== null);
const modifierOn = computed(
  () => props.activatesAt < props.duration && props.activatesAt >= 0,
);
// Deterministic vertical jitter so overlapping sim times spread into a band.
const simDots = computed(() =>
  (props.simTimes ?? []).map((t, i) => ({
    x: tx(t),
    y: AXIS_Y + (((i * 2654435761) >>> 8) % 17) - 8,
  })),
);

function fmt(v: number): string {
  return Number.isFinite(v) ? (Math.round(v * 100) / 100).toString() : "—";
}
</script>

<template>
  <div ref="el" class="mtl-wrap">
    <svg :width="width" :height="HEIGHT" class="mtl-svg" role="img"
      aria-label="Infection event timeline">
      <line :x1="tx(0)" :y1="AXIS_Y" :x2="tx(duration)" :y2="AXIS_Y" stroke="var(--color-border)"
        stroke-width="2" />
      <text :x="tx(0)" :y="AXIS_Y + 22" text-anchor="start" class="mtl-label">0</text>
      <text :x="tx(duration)" :y="AXIS_Y + 22" text-anchor="end" class="mtl-label">
        {{ fmt(duration) }}
      </text>

      <template v-if="modifierOn">
        <line :x1="tx(activatesAt)" :y1="AXIS_Y - 14" :x2="tx(activatesAt)" :y2="AXIS_Y + 10"
          class="mtl-activation" stroke-width="1.5" stroke-dasharray="3 2" />
        <text :x="tx(activatesAt)" :y="AXIS_Y - 17" text-anchor="middle" class="mtl-label mtl-activation-label">
          modifier on
        </text>
      </template>

      <template v-if="simMode">
        <circle v-for="(dot, i) in simDots" :key="i" :cx="dot.x" :cy="dot.y" r="3.5" class="mtl-sim-dot"
          :style="{ fill: color }" />
      </template>
      <template v-else>
        <circle v-for="(t, i) in discarded" :key="'d' + i" :cx="tx(t)" :cy="AXIS_Y" r="5" class="mtl-discarded">
          <title>{{ discardedLabel }} at t = {{ fmt(t) }}</title>
        </circle>
        <circle v-for="(t, i) in events" :key="'e' + i" :cx="tx(t)" :cy="AXIS_Y" r="5" :fill="color"
          class="mtl-event">
          <title>infection at t = {{ fmt(t) }}</title>
        </circle>
        <template v-if="recovered">
          <line :x1="tx(duration)" :y1="AXIS_Y - 9" :x2="tx(duration)" :y2="AXIS_Y + 9" stroke="#dc2626"
            stroke-width="2" />
          <text :x="tx(duration)" :y="AXIS_Y - 12" text-anchor="end" class="mtl-label mtl-recovered">
            recovered
          </text>
        </template>
      </template>
    </svg>
  </div>
</template>

<style scoped>
.mtl-wrap {
  width: 100%;
}

.mtl-svg {
  display: block;
  overflow: visible;
}

.mtl-label {
  font-size: 11px;
  fill: var(--color-text-secondary);
}

.mtl-activation {
  stroke: #d97706;
}

.mtl-activation-label {
  fill: #d97706;
  font-weight: 600;
}

.mtl-recovered {
  fill: #dc2626;
}

.mtl-sim-dot {
  opacity: 0.35;
}

.mtl-discarded {
  fill: var(--color-bg-0, #fff);
  stroke: #9ca3af;
  stroke-width: 1.5;
  stroke-dasharray: 2 1.5;
}
</style>
