<script setup lang="ts">
import { computed, ref, watch, onMounted, onBeforeUnmount } from "vue";
import { Button, Hint, Toggle } from "cfasim-ui/components";
import { LineChart, BarChart } from "cfasim-ui/charts";
import type { InfectionRate } from "../composables/infectionRate";
import {
  effectiveRateCurve,
  libraryOverlay,
  inverseCumulativeRate,
  sampledCumulative,
  expFromUniform,
  simulateInfectionRuns,
  eventTimeHistogram,
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
  // The assigned curve changed → the drawn τ's and the pooled samples (tied to
  // this individual's curve) no longer apply.
  clearDraws();
  clearSim();
  pooledTimes.value = [];
  mode.value = "draw";
}

// A constant rate is a homogeneous Poisson process: the inter-event times Δτ
// are simply Exp(rate). By default we present it that way (Δτ ~ Exp(rate),
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
  /** Inter-event time Δτ = τ_k − τ_{k−1} this draw buys (the Δτ column). */
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

// Two separate things:
//   • The TIMELINE shows only the CURRENT round — either the manual
//     individual's draws (draw mode) or the latest Simulate 100× round (sim
//     mode). Starting a new round replaces it.
//   • The DISTRIBUTION histogram below pools EVERY event time across all rounds
//     (`pooledTimes`); it keeps growing until Reset.
const mode = ref<"draw" | "sim">("draw");
const pooledTimes = ref<number[]>([]); // all rounds → the histogram
const simRound = ref<number[]>([]); // current sim round's revealed times → timeline

// Draw-mode timeline reads the current individual (the `draws` walk).
const infectionCount = computed(() => validEvents.value.length);
const currentTime = computed(() => {
  if (exhausted.value) return curve.value.duration;
  const v = validEvents.value;
  return v.length ? (v[v.length - 1].tau as number) : 0;
});

// Draw next samples one increment for the current individual and adds its
// event time to the running distribution. Once that individual recovers, the
// next click starts a fresh one — so Draw next always adds to the total. It
// also switches the timeline back to draw mode (off the last sim round).
function drawNext() {
  if (!hasArea.value || simAnimating.value) return;
  if (mode.value === "sim") {
    simRound.value = [];
    mode.value = "draw";
  }
  let base = exhausted.value ? [] : draws.value; // recovered → start a new individual
  const next = [...base, expFromUniform(Math.random())];
  const cumulative = next.reduce((a, b) => a + b, 0);
  const tau = inverseCumulativeRate(curve.value, cumulative);
  draws.value = next;
  if (tau !== null) pooledTimes.value = [...pooledTimes.value, tau];
}

function reset() {
  clearDraws();
  clearSim();
  pooledTimes.value = [];
  mode.value = "draw";
}

// "Simulate 100×": one round of many full simulations. Its event times animate
// onto the timeline (replacing the previous round) and accumulate into the
// distribution alongside every earlier round/draw.
const SIM_RUNS = 100;
const SIM_ANIM_MS = 1600;
const simAnimating = ref(false);
let simRaf: number | null = null;

function clearDraws() {
  draws.value = [];
}
function clearSim() {
  if (simRaf !== null) {
    cancelAnimationFrame(simRaf);
    simRaf = null;
  }
  simAnimating.value = false;
  simRound.value = [];
}

function simulateMany() {
  if (!hasArea.value || simAnimating.value) return;
  clearSim();
  clearDraws(); // the manual individual is no longer the current round
  mode.value = "sim";
  const roundTimes = simulateInfectionRuns(curve.value, SIM_RUNS, Math.random).flat();
  if (!roundTimes.length) return;
  const poolBase = pooledTimes.value; // earlier rounds keep accumulating
  simAnimating.value = true;
  const start = performance.now();
  const step = () => {
    const frac = Math.min(1, (performance.now() - start) / SIM_ANIM_MS);
    const target = Math.max(1, Math.round(frac * roundTimes.length));
    const shown = roundTimes.slice(0, target);
    simRound.value = shown; // current round → timeline
    pooledTimes.value = [...poolBase, ...shown]; // all rounds → histogram
    if (frac < 1) {
      simRaf = requestAnimationFrame(step);
    } else {
      simRaf = null;
      simAnimating.value = false;
    }
  };
  simRaf = requestAnimationFrame(step);
}

const hasSamples = computed(() => pooledTimes.value.length > 0);
// The timeline shows only the current round; sim mode spans the full infectious
// period, draw mode fills in up to the current event.
const timelineVisible = computed(() =>
  mode.value === "sim" ? simRound.value.length > 0 : draws.value.length > 0,
);
const axisEnd = computed(() =>
  mode.value === "sim" ? curve.value.duration : currentTime.value,
);

// A new rate invalidates the drawn τ's (different Λ), so start fresh. When the
// library's curve set changes (or we switch into a library rate) assign a fresh
// random curve; plain scale edits keep the same assigned curve.
let prevLibrarySig: string | null = null;
watch(
  () => props.modelValue,
  (rate) => {
    clearDraws();
    clearSim();
    pooledTimes.value = []; // a different Λ invalidates the sampled times
    mode.value = "draw";
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
const RED = "#dc2626";

// Bigger axis text, passed as props so each chart reserves the right gutter.
const axisTextStyle = { fontSize: 13 };

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

// Library only: every per-person curve (faint blue) + the population mean
// (red), with the currently-assigned curve drawn bold on top — same normalized
// axes as the r(t) chart above. The mean is series[0] so the tooltip binds to
// it (it spans the full τ range, unlike the individual curves).
const overlaySeries = computed(() => {
  const o = libraryOverlay(props.modelValue);
  const series: Array<{
    x: number[];
    data: number[];
    color: string;
    opacity?: number;
    strokeWidth?: number;
    showInTooltip?: boolean;
  }> = [];
  if (o.mean.x.length) {
    series.push({ x: o.mean.x, data: o.mean.data, color: RED, strokeWidth: 2 });
  }
  o.curves.forEach((c, i) => {
    if (i === libraryIndex.value) return; // the assigned curve is drawn on top
    series.push({
      x: c.x,
      data: c.lambda,
      color: BLUE,
      opacity: 0.22,
      strokeWidth: 1,
      showInTooltip: false,
    });
  });
  const assigned = o.curves[libraryIndex.value];
  if (assigned) {
    series.push({
      x: assigned.x,
      data: assigned.lambda,
      color: BLUE,
      strokeWidth: 2,
      showInTooltip: false,
    });
  }
  return series;
});

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

// Distribution of all sampled event times, binned over [0, duration]. The
// histogram approaches the rate curve r(t) as samples accumulate.
const DIST_BINS = 20;
const timeHistogram = computed(() =>
  eventTimeHistogram(pooledTimes.value, curve.value.duration, DIST_BINS),
);
// Plot the percentage per bin (not raw counts) so the y-axis stays bounded as
// samples accumulate.
const distSeries = computed(() => [
  { data: timeHistogram.value.percentages, color: PURPLE },
]);
/** Linear interpolation of the effective λ(τ) at a single time. */
function lambdaAt(t: number): number {
  const { x, lambda } = curve.value;
  if (!x.length) return 0;
  if (t <= x[0]) return lambda[0];
  for (let i = 1; i < x.length; i++) {
    if (t <= x[i]) {
      const span = x[i] - x[i - 1];
      if (span === 0) return lambda[i];
      return lambda[i - 1] + ((t - x[i - 1]) / span) * (lambda[i] - lambda[i - 1]);
    }
  }
  return lambda[lambda.length - 1];
}
// The expected shape r(t), sampled at the bin centers and overlaid on the bars
// (a BarChart summary line auto-scales to its own extent) so you can watch the
// sampled distribution converge to the rate curve.
const distOverlay = computed(() => {
  const h = timeHistogram.value;
  return [
    {
      x: h.centers.map((_, i) => i),
      data: h.centers.map((c) => lambdaAt(c)),
      color: BLUE,
      strokeWidth: 2,
      dots: false,
    },
  ];
});
// Thin the bin labels (20 bins would crowd the τ axis) — show every 5th.
function distLabel(label: string, index: number): string {
  return index % 5 === 0 ? label : "";
}

// --- Hand-drawn SVG timeline -------------------------------------------
// Fixed [0, duration] scale; every pooled event time is a dot, accumulating
// across draws and simulations.
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
onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  if (simRaf !== null) cancelAnimationFrame(simRaf);
});

/** Map a time t to an x pixel position on the timeline. */
function tx(t: number): number {
  const d = curve.value.duration || 1;
  const inner = Math.max(1, timelineWidth.value - PAD_L - PAD_R);
  const frac = d > 0 ? Math.min(1, Math.max(0, t / d)) : 0;
  return PAD_L + frac * inner;
}
// Draw-mode timeline: one solid dot per infection time of the current
// individual.
const dotPositions = computed(() =>
  validEvents.value.map((ev) => ({ x: tx(ev.tau as number), tau: ev.tau as number })),
);
// Sim-mode timeline: a dot per event time in the current round, with a
// deterministic vertical jitter so overlapping times spread into a band — the
// translucent overlap reads as the density of when infections happen (∝ r(t)).
const simDotPositions = computed(() =>
  simRound.value.map((t, i) => ({
    x: tx(t),
    y: AXIS_Y + (((i * 2654435761) >>> 8) % 17) - 8,
  })),
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
          hint="Show the general inverse-CDF method (Λ⁻¹) instead of drawing each Δτ straight from Exp(rate)."
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
          :tick-label-style="axisTextStyle" :axis-label-style="axisTextStyle"
          x-label="t (days since infected)" y-label="r(t)" tooltip-trigger="hover">
          <template #tooltip="{ xLabel, values }">
            <div class="explorer-tooltip">
              <div v-if="xLabel != null">t = {{ fmtTau(xLabel) }}</div>
              <div>r = {{ fmtTau(values[0]?.value) }}</div>
            </div>
          </template>
        </LineChart>

        <!-- Library only: the whole population of curves with the mean in red,
             so the assigned curve above can be read in context. -->
        <div v-if="isLibrary" class="explorer-overlay">
          <h4>Library curves <span class="explorer-overlay-mean">(mean)</span></h4>
          <p class="explorer-hint">
            Every per-person curve in the library, with the population mean in
            <span class="explorer-overlay-mean">red</span>. The bold blue curve
            is the one assigned above.
          </p>
          <LineChart :series="overlaySeries" :height="180" :y-min="0" :menu="false"
            :tick-label-style="axisTextStyle" :axis-label-style="axisTextStyle"
            x-label="t (days since infected)" y-label="r(t)" tooltip-trigger="hover">
            <template #tooltip="{ xLabel, values }">
              <div class="explorer-tooltip">
                <div v-if="xLabel != null">t = {{ fmtTau(xLabel) }}</div>
                <div>mean = {{ fmtTau(values[0]?.value) }}</div>
              </div>
            </template>
          </LineChart>
        </div>

        <!-- Distribution of simulated event times: pooled across all draws +
             simulations, binned over [0, duration]. The bars approach r(t) as
             samples grow. -->
        <div v-if="hasSamples" class="explorer-sim">
          <h3>Distribution of simulated event times</h3>
          <p class="explorer-hint">
            {{ timeHistogram.total }} event time{{ timeHistogram.total === 1 ? "" : "s" }}
            sampled across all rounds, binned over the infectious period. The bars
            approach the rate curve <span class="explorer-rate-line">r(t)</span> as
            more are sampled.
          </p>
          <BarChart :categories="timeHistogram.categories" :series="distSeries" :summary-lines="distOverlay"
            :height="200" :menu="false" x-label="t (days since infected)" y-label="% of samples"
            :tick-label-style="axisTextStyle" :axis-label-style="axisTextStyle"
            value-tick-format="%.0f%%" :category-format="distLabel">
            <template #tooltip="{ category, values }">
              <div class="explorer-tooltip">
                <div>t ≈ {{ category }}</div>
                <div>{{ fmtTau(values[0]?.value) }}% of samples</div>
              </div>
            </template>
          </BarChart>
        </div>
      </div>

      <!-- Right: inverse-CDF sampling. Draw Exp(1) deltas, accumulate them,
           and invert through Λ(τ) to event times. -->
      <div class="explorer-panel">
        <div class="explorer-controls">
          <h3>{{ simplifiedView ? "Event times" : "Cumulative rate c(t)" }}</h3>
          <div class="explorer-buttons">
            <Button :disabled="!hasArea || simAnimating" @click="drawNext">Draw next</Button>
            <Button variant="secondary" :disabled="!hasArea || simAnimating" @click="simulateMany">Simulate 100×</Button>
            <Button variant="secondary" :disabled="!draws.length && !hasSamples" @click="reset">Reset</Button>
          </div>
        </div>
        <!-- Simplified homogeneous view (constant rate). -->
        <p v-if="simplifiedView" class="explorer-hint">
          A constant rate is a homogeneous Poisson process: the time Δτ before each event is drawn from
          <strong>Exp({{ fmt(constantRate) }})</strong> (mean {{ fmt(1 / constantRate) }} days), and τ
          is the elapsed time since the person was infected.
        </p>

        <!-- General inverse-CDF view (used by every time-varying rate). -->
        <template v-else>
          <p class="explorer-hint">{{ defs.c }}</p>
          <LineChart :series="cumSeries" :height="200" :y-min="0" :menu="false" x-label="t (days since infected)"
            :tick-label-style="axisTextStyle" :axis-label-style="axisTextStyle"
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

        <!-- Timeline: only the CURRENT round — the manual individual (draw mode)
             or the latest Simulate round (sim mode). The distribution below
             keeps every round. -->
        <p v-show="timelineVisible" class="timeline-title">
          <template v-if="mode === 'sim'">
            This round: {{ SIM_RUNS }} individuals · {{ simRound.length }}
            infection{{ simRound.length === 1 ? "" : "s" }}<span v-if="simAnimating"
              class="timeline-title-sim"> · simulating…</span>
          </template>
          <template v-else>
            {{ infectionCount }} infection{{ infectionCount === 1 ? "" : "s" }}
            over {{ fmt(currentTime) }} days<span v-if="exhausted" class="timeline-title-recovered">
              · recovered</span>
          </template>
        </p>
        <div v-show="timelineVisible" ref="timelineEl" class="timeline-wrap">
          <svg :width="timelineWidth" :height="TIMELINE_H" class="timeline-svg" role="img"
            aria-label="Infection event timeline">
            <line :x1="tx(0)" :y1="AXIS_Y" :x2="tx(axisEnd)" :y2="AXIS_Y" stroke="var(--color-border)"
              stroke-width="2" />
            <text :x="tx(0)" :y="AXIS_Y + 22" text-anchor="start" class="timeline-label">
              0
            </text>

            <!-- Sim round: jittered translucent dots over the full period. -->
            <template v-if="mode === 'sim'">
              <circle v-for="(dot, i) in simDotPositions" :key="i" :cx="dot.x" :cy="dot.y" r="3.5"
                class="sim-dot" />
              <text v-if="curve.duration > 0" :x="tx(curve.duration)" :y="AXIS_Y + 22" text-anchor="end"
                class="timeline-label">
                {{ fmt(curve.duration) }}
              </text>
            </template>

            <!-- Draw round: the current individual's solid dots + recovery. -->
            <template v-else>
              <text v-if="currentTime > 0 && !exhausted" :x="tx(currentTime)" :y="AXIS_Y + 22"
                text-anchor="middle" class="timeline-label">
                {{ fmt(currentTime) }}
              </text>
              <circle v-for="(dot, i) in dotPositions" :key="i" :cx="dot.x" :cy="AXIS_Y" r="5" fill="#7c3aed">
                <title>infection at t = {{ fmt(dot.tau) }}</title>
              </circle>
              <template v-if="exhausted">
                <line :x1="tx(curve.duration)" :y1="AXIS_Y - 9" :x2="tx(curve.duration)" :y2="AXIS_Y + 9"
                  stroke="#dc2626" stroke-width="2" />
                <text :x="tx(curve.duration)" :y="AXIS_Y + 22" text-anchor="end"
                  class="timeline-label timeline-recovery">
                  recovered · {{ fmt(curve.duration) }}
                </text>
              </template>
            </template>
          </svg>
        </div>

        <!-- The current individual's inverse-CDF walk (e ~ Exp(1), Δτ, τ). Each
             Draw next extends it; once recovered, the next Draw next starts a
             fresh individual. -->
        <table v-if="events.length" class="explorer-table">
          <thead>
            <tr v-if="simplifiedView">
              <th>Δτ ~ Exp(rate)</th>
              <th>
                τ = Σ
                <Hint text="Event time since infection: the running sum of the Δτ's so far (τ = Σ Δτ)." />
              </th>
            </tr>
            <tr v-else>
              <th>e ~ Exp(1)</th>
              <th>
                Δτ
                <Hint
                  text="Time-scaled inter-event time Δτ = d(c+e) − d(c). Invert the cumulative rate at the new running total c+e, then subtract the previous event time d(c)." />
              </th>
              <th>
                τ = Σ
                <Hint text="Event time since infection: the running sum of the Δτ's so far (τ = Σ Δτ)." />
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
        <p v-if="exhausted" class="explorer-hint">
          {{
            simplifiedView
              ? "The accumulated time passed the infectious period — this individual recovers."
              : "The cumulative draw exceeded the curve's total area — this individual recovers."
          }}
          Press <strong>Draw next</strong> to start a new individual.
        </p>
        <p v-if="!hasSamples && !events.length" class="explorer-hint explorer-empty">
          No samples yet — press <strong>Draw next</strong> to sample one time, or
          <strong>Simulate 100×</strong> to build the distribution quickly.
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

.explorer-overlay,
.explorer-sim {
  display: flex;
  flex-direction: column;
  gap: 0.4em;
  margin-top: 0.8em;
  padding-top: 0.8em;
  border-top: 1px solid var(--color-border);
}

.explorer-sim h3 {
  margin: 0;
  font-size: var(--font-size-md, 1rem);
}

.explorer-overlay h4 {
  margin: 0;
  font-size: var(--font-size-md, 1rem);
}

.explorer-overlay-mean {
  color: #dc2626;
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

.timeline-title-sim {
  color: #7c3aed;
  font-weight: 600;
}

.timeline-title-recovered {
  color: #dc2626;
  font-weight: 600;
}

.timeline-recovery {
  fill: #dc2626;
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

/* Pooled event-time dots: translucent so overlapping times read as density. */
.sim-dot {
  fill: #7c3aed;
  opacity: 0.35;
}

.explorer-rate-line {
  color: #2563eb;
  font-weight: 600;
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
