<script setup lang="ts">
import { computed, ref, watch, onMounted, onBeforeUnmount } from "vue";
import { Button, Hint, Toggle } from "cfasim-ui/components";
import { LineChart } from "cfasim-ui/charts";
import type { InfectionRate } from "../composables/infectionRate";
import {
  effectiveRateCurve,
  inverseCumulativeRate,
  sampledCumulative,
  expFromUniform,
  describeRate,
  rateFunctionDefs,
} from "../composables/rateExplorer";

const props = defineProps<{ modelValue: InfectionRate }>();

// Library rates assign each person a random curve at setup; the explorer shows
// one such assigned curve, re-rollable via "New random curve". The other rate
// types are shared by every individual, so there's nothing to assign.
const isLibrary = computed(() => props.modelValue.type === "library");
const libraryCount = computed(() =>
  props.modelValue.type === "library" ? props.modelValue.rates.length : 0,
);
const libraryIndex = ref(0);

const curve = computed(() =>
  effectiveRateCurve(props.modelValue, libraryIndex.value),
);
const desc = computed(() => describeRate(props.modelValue));
const defs = computed(() => rateFunctionDefs(props.modelValue));

/** A random curve index, preferring one different from the current pick. */
function randomLibraryIndex(): number {
  const n = libraryCount.value;
  if (n <= 1) return 0;
  let i = Math.floor(Math.random() * n);
  if (i === libraryIndex.value) i = (i + 1) % n;
  return i;
}

function assignRandomCurve() {
  libraryIndex.value = randomLibraryIndex();
  draws.value = []; // the assigned curve changed → drawn τ's no longer apply
}

// A constant rate is a homogeneous Poisson process: the gaps between events
// are simply Exp(rate). By default we present it that way (gap ~ Exp(rate),
// τ = running sum); toggling "Generalized" on shows the general inverse-CDF
// method — the same path the time-varying rates use. The toggle only applies
// to a constant rate; everything else has no single rate and always uses the
// general view.
const isConstant = computed(() => props.modelValue.type === "constant");
const constantRate = computed(() =>
  props.modelValue.type === "constant" ? props.modelValue.value : 0,
);
const generalized = ref(false);
const simplifiedView = computed(() => isConstant.value && !generalized.value);

// Exp(1) increments the user has drawn. Each one advances the cumulative
// hazard E; the next event time is τ = Λ⁻¹(E) (null once the curve is
// exhausted → the person recovered before another event).
const draws = ref<number[]>([]);

interface DemoEvent {
  /** Exp(1) draw δ (event-space increment). */
  delta: number;
  /** Running Σδ in event space — drives the c(t) chart's y-position. */
  cumulative: number;
  /** Time-scaled δ: the time increment τ_k − τ_{k−1} this draw buys. */
  scaledDelta: number | null;
  /** Event time τ = Σ of the time-scaled δ's. `null` once recovered. */
  tau: number | null;
}

const events = computed<DemoEvent[]>(() => {
  const out: DemoEvent[] = [];
  let cumulative = 0;
  let prevTau = 0;
  for (const delta of draws.value) {
    cumulative += delta;
    const tau = inverseCumulativeRate(curve.value, cumulative);
    const scaledDelta = tau === null ? null : tau - prevTau;
    out.push({ delta, cumulative, scaledDelta, tau });
    if (tau !== null) prevTau = tau;
  }
  return out;
});

const validEvents = computed(() => events.value.filter((ev) => ev.tau !== null));
const exhausted = computed(() => {
  const last = events.value[events.value.length - 1];
  return last ? last.tau === null : false;
});
const hasArea = computed(() => curve.value.total > 0);

// Number of infections so far, and the process's "current time": the latest
// event time, or — once a draw overshoots the curve — the deterministic
// recovery time (the end of the infectious period).
const infectionCount = computed(() => validEvents.value.length);
const lastTau = computed(() => {
  const v = validEvents.value;
  return v.length ? (v[v.length - 1].tau as number) : 0;
});
const currentTime = computed(() =>
  exhausted.value ? curve.value.duration : lastTau.value,
);

function drawNext() {
  if (exhausted.value || !hasArea.value) return;
  draws.value = [...draws.value, expFromUniform(Math.random())];
}
function reset() {
  draws.value = [];
}

// A new rate invalidates the drawn τ's (different Λ), so start fresh. When the
// library's curve set changes (or we switch into a library rate) assign a fresh
// random curve; plain scale edits keep the same assigned curve.
let prevLibrarySig: string | null = null;
watch(
  () => props.modelValue,
  (rate) => {
    draws.value = [];
    const sig = rate.type === "library" ? `library:${rate.rates.length}` : null;
    if (sig && sig !== prevLibrarySig) libraryIndex.value = randomLibraryIndex();
    prevLibrarySig = sig;
  },
  { immediate: true },
);

function fmt(v: number): string {
  return Number.isFinite(v) ? (Math.round(v * 1000) / 1000).toString() : "—";
}

const BLUE = "#2563eb";
const PURPLE = "#7c3aed";
const GUIDE = "#9ca3af";

// Left: λ(τ) with the area underneath shaded (∫λ = R₀).
const rateSeries = computed(() => [
  { x: curve.value.x, data: curve.value.lambda, color: BLUE, strokeWidth: 2 },
]);
const rateAreas = computed(() => [
  {
    x: curve.value.x,
    upper: curve.value.lambda,
    lower: curve.value.lambda.map(() => 0),
    color: BLUE,
    opacity: 0.18,
  },
]);

// Right: Λ(τ) plus event markers and guide lines for the latest draw,
// showing how an Exp(1) increment on the y-axis inverts to a τ on the x.
const cumSeries = computed(() => {
  // Dense resampling so the rendered (straight-chord) polyline tracks the true
  // piecewise-quadratic c(t) — otherwise event markers at (τ, E) sit off the
  // line. See `sampledCumulative`.
  const display = sampledCumulative(curve.value);
  const series: Array<{
    x: number[];
    data: number[];
    color: string;
    strokeWidth?: number;
    dashed?: boolean;
    dots?: boolean;
    line?: boolean;
    showInTooltip?: boolean;
  }> = [{ x: display.x, data: display.data, color: BLUE, strokeWidth: 2 }];
  const last = validEvents.value[validEvents.value.length - 1];
  if (last && last.tau !== null) {
    // Horizontal: E events, from τ=0 to the inverted τ.
    series.push({
      x: [0, last.tau],
      data: [last.cumulative, last.cumulative],
      color: GUIDE,
      strokeWidth: 1,
      dashed: true,
      showInTooltip: false,
    });
    // Vertical: drop from the curve down to the τ axis.
    series.push({
      x: [last.tau, last.tau],
      data: [last.cumulative, 0],
      color: GUIDE,
      strokeWidth: 1,
      dashed: true,
      showInTooltip: false,
    });
  }
  // Event markers at (τ_i, E_i) on the curve.
  if (validEvents.value.length) {
    series.push({
      x: validEvents.value.map((ev) => ev.tau as number),
      data: validEvents.value.map((ev) => ev.cumulative),
      color: PURPLE,
      line: false,
      dots: true,
      showInTooltip: false,
    });
  }
  return series;
});

// --- Hand-drawn SVG timeline -------------------------------------------
// Fixed [0, duration] scale, but the axis is only drawn up to the current
// time — it fills in as events elapse, ending at the recovery marker.
const TIMELINE_H = 50;
// No left pad so t=0 lines up flush with the title / panel left edge; a right
// pad keeps the end labels from clipping.
const PAD_L = 0;
const PAD_R = 28;
const AXIS_Y = 20;

const timelineEl = ref<HTMLElement | null>(null);
const timelineWidth = ref(560);
let resizeObserver: ResizeObserver | null = null;
onMounted(() => {
  resizeObserver = new ResizeObserver((entries) => {
    const w = entries[0]?.contentRect.width;
    if (w) timelineWidth.value = w;
  });
  if (timelineEl.value) resizeObserver.observe(timelineEl.value);
});
onBeforeUnmount(() => resizeObserver?.disconnect());

/** Map a time t to an x pixel position on the timeline. */
function tx(t: number): number {
  const d = curve.value.duration || 1;
  const inner = Math.max(1, timelineWidth.value - PAD_L - PAD_R);
  const frac = d > 0 ? Math.min(1, Math.max(0, t / d)) : 0;
  return PAD_L + frac * inner;
}
const dotPositions = computed(() =>
  validEvents.value.map((ev) => ({ x: tx(ev.tau as number), tau: ev.tau as number })),
);

function fmtTau(v: unknown): string {
  const n = Number(v);
  return Number.isFinite(n)
    ? n.toLocaleString(undefined, { maximumFractionDigits: 2 })
    : "—";
}
</script>

<template>
  <section class="explorer">
    <header class="explorer-header">
      <h2>Intrinsic Infectiousness</h2>
      <div class="explorer-subline">
        <p class="explorer-subtitle">{{ desc.title }}</p>
        <!-- Constant rate only: switch the right panel between the simple
             homogeneous-Poisson view and the general inverse-CDF method. -->
        <Toggle v-if="isConstant" :model-value="generalized" label="Generalized"
          hint="Show the general inverse-CDF method (Λ⁻¹) instead of drawing gaps straight from Exp(rate)."
          class="explorer-mode-toggle" @update:model-value="(v: boolean) => (generalized = v)" />
      </div>
      <!-- Library rates assign each person a random curve; everything else is
           shared by all individuals. -->
      <div class="explorer-note">
        <template v-if="isLibrary">
          <p class="explorer-note-text">
            Each individual is assigned a random curve from the library — showing
            curve #{{ libraryIndex + 1 }} of {{ libraryCount }} (R₀ ≈ {{ fmt(curve.total) }}).
          </p>
          <Button variant="secondary" @click="assignRandomCurve">New random curve</Button>
        </template>
        <p v-else class="explorer-note-text">Every individual gets the same rate function.</p>
      </div>
    </header>

    <div class="explorer-grid">
      <!-- Left: the rate function λ(τ) and its area (= expected attempts). -->
      <div class="explorer-panel">
        <h3>Rate function r(t)</h3>
        <p class="explorer-hint">{{ defs.r }}</p>
        <LineChart :series="rateSeries" :areas="rateAreas" :height="240" :y-min="0" :menu="false"
          x-label="t (days since infected)" y-label="r(t)" tooltip-trigger="hover">
          <template #tooltip="{ xLabel, values }">
            <div class="explorer-tooltip">
              <div v-if="xLabel != null">t = {{ fmtTau(xLabel) }}</div>
              <div>r = {{ fmtTau(values[0]?.value) }}</div>
            </div>
          </template>
        </LineChart>
      </div>

      <!-- Right: inverse-CDF sampling. Draw Exp(1) deltas, accumulate them,
           and invert through Λ(τ) to event times. -->
      <div class="explorer-panel">
        <div class="explorer-controls">
          <h3>{{ simplifiedView ? "Event times" : "Cumulative rate c(t)" }}</h3>
          <div class="explorer-buttons">
            <Button :disabled="exhausted || !hasArea" @click="drawNext">Draw next</Button>
            <Button variant="secondary" :disabled="!draws.length" @click="reset">Reset</Button>
          </div>
        </div>
        <!-- Simplified homogeneous view (constant rate). -->
        <p v-if="simplifiedView" class="explorer-hint">
          A constant rate is a homogeneous Poisson process: the gap before each event is drawn from
          <strong>Exp({{ fmt(constantRate) }})</strong> (mean {{ fmt(1 / constantRate) }} days), and τ
          is the running sum of those gaps.
        </p>

        <!-- General inverse-CDF view (used by every time-varying rate). -->
        <template v-else>
          <p class="explorer-hint">{{ defs.c }}</p>
          <LineChart :series="cumSeries" :height="200" :y-min="0" :menu="false" x-label="t (days since infected)"
            y-label="c(t)" tooltip-trigger="hover">
            <template #tooltip="{ xLabel, values }">
              <div class="explorer-tooltip">
                <div v-if="xLabel != null">t = {{ fmtTau(xLabel) }}</div>
                <div>c = {{ fmtTau(values[0]?.value) }}</div>
              </div>
            </template>
          </LineChart>

          <h3>Inverse cumulative rate d(t)</h3>
          <p class="explorer-hint explorer-def-d">{{ defs.d }}</p>
        </template>

        <p v-show="draws.length" class="timeline-title">
          {{ infectionCount }} total infection{{ infectionCount === 1 ? "" : "s" }}
          over {{ fmt(currentTime) }} days<span v-if="exhausted" class="timeline-title-recovered">
            · recovered</span>
        </p>
        <div v-show="draws.length" ref="timelineEl" class="timeline-wrap">
          <svg :width="timelineWidth" :height="TIMELINE_H" class="timeline-svg" role="img"
            aria-label="Infection event timeline">
            <!-- Elapsed axis only — the future isn't drawn until it elapses. -->
            <line :x1="tx(0)" :y1="AXIS_Y" :x2="tx(currentTime)" :y2="AXIS_Y" stroke="var(--color-border)"
              stroke-width="2" />
            <text :x="tx(0)" :y="AXIS_Y + 22" text-anchor="start" class="timeline-label">
              0
            </text>
            <!-- Current-time tick (suppressed once recovered — the recovery
                 label takes that spot). -->
            <text v-if="currentTime > 0 && !exhausted" :x="tx(currentTime)" :y="AXIS_Y + 22" text-anchor="middle"
              class="timeline-label">
              {{ fmt(currentTime) }}
            </text>
            <!-- One solid dot per infection. -->
            <circle v-for="(dot, i) in dotPositions" :key="i" :cx="dot.x" :cy="AXIS_Y" r="5" fill="#7c3aed">
              <title>infection at t = {{ fmt(dot.tau) }}</title>
            </circle>
            <!-- Recovery marker at the deterministic end of infectiousness. -->
            <template v-if="exhausted">
              <line :x1="tx(curve.duration)" :y1="AXIS_Y - 9" :x2="tx(curve.duration)" :y2="AXIS_Y + 9" stroke="#dc2626"
                stroke-width="2" />
              <text :x="tx(curve.duration)" :y="AXIS_Y + 22" text-anchor="end" class="timeline-label timeline-recovery">
                recovered · {{ fmt(curve.duration) }}
              </text>
            </template>
          </svg>
        </div>

        <table v-if="events.length" class="explorer-table">
          <thead>
            <tr v-if="simplifiedView">
              <th>gap ~ Exp(rate)</th>
              <th>
                τ = Σ
                <Hint text="Event time since infection: the running sum of the gaps so far (τ = Σ gap)." />
              </th>
            </tr>
            <tr v-else>
              <th>e ~ Exp(1)</th>
              <th>
                gap
                <Hint
                  text="Time-scaled inter-event gap: d(c+e) − d(c). Invert the cumulative rate at the new running total c+e, then subtract the previous event time d(c)." />
              </th>
              <th>
                τ = Σ
                <Hint text="Event time since infection: the running sum of the gaps so far (τ = Σ gap)." />
              </th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(ev, i) in events" :key="i">
              <td v-if="!simplifiedView" class="num">{{ fmt(ev.delta) }}</td>
              <td class="num">
                <span v-if="ev.scaledDelta !== null">{{ fmt(ev.scaledDelta) }}</span>
                <span v-else class="recovered">—</span>
              </td>
              <td class="num">
                <span v-if="ev.tau !== null">{{ fmt(ev.tau) }}</span>
                <span v-else class="recovered">recovered</span>
              </td>
            </tr>
          </tbody>
        </table>
        <p v-else class="explorer-hint explorer-empty">
          No draws yet — press <strong>Draw next</strong> to sample the first
          infection time.
        </p>
        <p v-if="exhausted" class="explorer-hint">
          {{
            simplifiedView
              ? "The accumulated time passed the infectious period — the person recovers before the next event."
              : "The cumulative draw exceeded the curve's total area — the person recovers before the next attempt."
          }}
        </p>
      </div>
    </div>
  </section>
</template>

<style scoped>
.explorer {
  display: flex;
  flex-direction: column;
  gap: 1em;
}

.explorer-header {
  display: flex;
  flex-direction: column;
  gap: 0.2em;
}

.explorer-header h2 {
  margin: 0;
}

/* Subtitle and the (constant-rate) view toggle share a row, wrapping the
   toggle below the description when there isn't room beside it. */
.explorer-subline {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 0.4em 1em;
}

.explorer-mode-toggle {
  flex-shrink: 0;
}

.explorer-subtitle {
  margin: 0;
  color: var(--color-text-secondary);
}

/* The per-rate note ("same rate function" / library assignment + re-roll). */
.explorer-note {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.4em 0.8em;
}

.explorer-note-text {
  margin: 0;
  font-size: var(--font-size-sm, 0.875rem);
  color: var(--color-text-secondary);
}

.explorer-hint strong {
  color: var(--color-text);
  font-variant-numeric: tabular-nums;
}

.explorer-def-d {
  margin-top: 0.6em;
}

.explorer-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 1.5em;
}

@media (max-width: 900px) {
  .explorer-grid {
    grid-template-columns: 1fr;
  }
}

.explorer-panel {
  display: flex;
  flex-direction: column;
  gap: 0.4em;
  min-width: 0;
}

.explorer-panel h3 {
  margin: 0;
  font-size: var(--font-size-md, 1rem);
}

.explorer-hint {
  margin: 0;
  font-size: var(--font-size-sm, 0.875rem);
  color: var(--color-text-secondary);
}

.explorer-empty {
  margin-top: 0.5em;
}

.explorer-controls {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5em;
  flex-wrap: wrap;
}

.explorer-buttons {
  display: flex;
  gap: 0.4em;
}

.timeline-title {
  margin: 0.6em 0 0;
  font-size: var(--font-size-sm, 0.875rem);
  font-weight: 600;
  color: var(--color-text);
}

.timeline-title-recovered {
  color: #dc2626;
  font-weight: 600;
}

.timeline-wrap {
  width: 100%;
}

.timeline-svg {
  display: block;
  overflow: visible;
}

.timeline-label {
  font-size: 11px;
  fill: var(--color-text-secondary);
}

.timeline-recovery {
  fill: #dc2626;
}

.explorer-table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--font-size-sm, 0.875rem);
  font-variant-numeric: tabular-nums;
}

.explorer-table th,
.explorer-table td {
  padding: 0.25em 0.5em;
  text-align: left;
  border-bottom: 1px solid var(--color-border);
}

.explorer-table th {
  color: var(--color-text-secondary);
  font-weight: 600;
}

.explorer-table .num {
  text-align: left;
}

.recovered {
  color: var(--color-text-secondary);
  font-style: italic;
}

.explorer-tooltip {
  display: flex;
  flex-direction: column;
  gap: 1px;
  font-size: 0.6875rem;
  white-space: nowrap;
}
</style>
