<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { Button, Hint, NumberInput } from "cfasim-ui/components";
import { LineChart, BarChart } from "cfasim-ui/charts";
import type { InfectionRate } from "../composables/infectionRate";
import {
  effectiveRateCurve,
  eventTimeHistogram,
  expFromUniform,
  describeRate,
  histogramBins,
  rateAtTime,
  sparseBinLabel,
} from "../composables/rateExplorer";
import {
  type ModifierSpec,
  type ThinningDraw,
  defaultModifierSpec,
  modifierActive,
  modifiedRateAt,
  modifiedRateCurve,
  needsRedraw,
  reschedulingAttempts,
  simulateReschedulingRuns,
  simulateThinningRuns,
  thinningApplies,
  thinningAttempts,
} from "../composables/modifierExplorer";
import ModifierTimeline from "./ModifierTimeline.vue";

const props = defineProps<{ modelValue: InfectionRate }>();

// A Library rate shows its first curve.
const curve = computed(() => effectiveRateCurve(props.modelValue, 0));
const desc = computed(() => describeRate(props.modelValue));
const hasArea = computed(() => curve.value.total > 0);

// Panel-local demo modifier, not tied to the sidebar's modifier configs. The
// activation time falls back to the rate's default until the user sets it, or
// again when a new rate's support no longer contains their value.
const userActivatesAt = ref<number | null>(null);
const activatesAt = computed<number>({
  get: () => {
    const u = userActivatesAt.value;
    return u !== null && u <= curve.value.duration
      ? u
      : defaultModifierSpec(curve.value.duration).activatesAt;
  },
  set: (v) => {
    userActivatesAt.value = v;
  },
});
const factor = ref(defaultModifierSpec(curve.value.duration).factor);

const spec = computed<ModifierSpec>(() => ({
  activatesAt: activatesAt.value,
  factor: factor.value,
}));
const modified = computed(() => modifiedRateCurve(curve.value, spec.value));
const active = computed(() => modifierActive(curve.value, spec.value));
// f > 1: r(t) no longer bounds the modified rate, so only rescheduling runs.
const thinningOk = computed(() => thinningApplies(spec.value));

// The current individual under each method (draw mode).
const thinDraws = ref<ThinningDraw[]>([]);
const reschedDraws = ref<number[]>([]);

const thinAttempts = computed(() =>
  thinningAttempts(curve.value, spec.value, thinDraws.value),
);
const thinEvents = computed(() =>
  thinAttempts.value
    .filter((a) => a.outcome === "accepted")
    .map((a) => a.tau as number),
);
const thinRejected = computed(() =>
  thinAttempts.value
    .filter((a) => a.outcome === "rejected")
    .map((a) => a.tau as number),
);
const thinExhausted = computed(
  () => thinAttempts.value[thinAttempts.value.length - 1]?.outcome === "recovered",
);

const reschedAttempts = computed(() =>
  reschedulingAttempts(curve.value, modified.value, spec.value, reschedDraws.value),
);
const reschedEvents = computed(() =>
  reschedAttempts.value
    .filter((a) => a.outcome === "event")
    .map((a) => a.tau as number),
);
const reschedCancelled = computed(() =>
  reschedAttempts.value
    .filter((a) => a.outcome === "cancelled")
    .map((a) => a.tau as number),
);
const reschedExhausted = computed(
  () =>
    reschedAttempts.value[reschedAttempts.value.length - 1]?.outcome === "recovered",
);

// Pooled across every round.
interface MethodStats {
  times: number[];
  /** Offspring count per completed individual. */
  counts: number[];
  /** Thinning rejections or rescheduling cancellations. */
  discarded: number;
}
const emptyStats = (): MethodStats => ({ times: [], counts: [], discarded: 0 });
const thinStats = ref<MethodStats>(emptyStats());
const reschedStats = ref<MethodStats>(emptyStats());

const mode = ref<"draw" | "sim">("draw");
const simRound = ref<{ thin: number[]; resched: number[] } | null>(null);
const SIM_RUNS = 100;

// One forecast per method per click; a recovered individual rolls over to a
// fresh one.
function drawNext() {
  if (!hasArea.value) return;
  if (mode.value === "sim") {
    simRound.value = null;
    mode.value = "draw";
  }
  if (thinningOk.value) {
    const base = thinExhausted.value ? [] : thinDraws.value;
    const next = [...base, { e: expFromUniform(Math.random()), u: Math.random() }];
    const attempts = thinningAttempts(curve.value, spec.value, next);
    const last = attempts[attempts.length - 1];
    thinDraws.value = next;
    const s = thinStats.value;
    if (last.outcome === "accepted") {
      thinStats.value = { ...s, times: [...s.times, last.tau as number] };
    } else if (last.outcome === "rejected") {
      thinStats.value = { ...s, discarded: s.discarded + 1 };
    } else {
      const n = attempts.filter((a) => a.outcome === "accepted").length;
      thinStats.value = { ...s, counts: [...s.counts, n] };
    }
  }
  {
    // A cancellation at t_m is redrawn in the same click, as the activation
    // plan does.
    let next = reschedExhausted.value ? [] : reschedDraws.value;
    let attempts;
    do {
      next = [...next, expFromUniform(Math.random())];
      attempts = reschedulingAttempts(curve.value, modified.value, spec.value, next);
      const last = attempts[attempts.length - 1];
      const s = reschedStats.value;
      if (last.outcome === "event") {
        reschedStats.value = { ...s, times: [...s.times, last.tau as number] };
      } else if (last.outcome === "cancelled") {
        reschedStats.value = { ...s, discarded: s.discarded + 1 };
      } else if (last.outcome === "recovered") {
        const n = attempts.filter((a) => a.outcome === "event").length;
        reschedStats.value = { ...s, counts: [...s.counts, n] };
      }
    } while (needsRedraw(attempts));
    reschedDraws.value = next;
  }
}

function simulateMany() {
  if (!hasArea.value) return;
  thinDraws.value = [];
  reschedDraws.value = [];
  mode.value = "sim";
  const thinRuns = thinningOk.value
    ? simulateThinningRuns(curve.value, spec.value, SIM_RUNS)
    : [];
  const reschedRuns = simulateReschedulingRuns(
    curve.value,
    modified.value,
    spec.value,
    SIM_RUNS,
  );
  const ts = thinStats.value;
  thinStats.value = {
    times: [...ts.times, ...thinRuns.flatMap((r) => r.accepted)],
    counts: [...ts.counts, ...thinRuns.map((r) => r.accepted.length)],
    discarded: ts.discarded + thinRuns.reduce((a, r) => a + r.rejected.length, 0),
  };
  const rs = reschedStats.value;
  reschedStats.value = {
    times: [...rs.times, ...reschedRuns.flatMap((r) => r.events)],
    counts: [...rs.counts, ...reschedRuns.map((r) => r.events.length)],
    discarded: rs.discarded + reschedRuns.reduce((a, r) => a + r.cancelled, 0),
  };
  simRound.value = {
    thin: thinRuns.flatMap((r) => r.accepted),
    resched: reschedRuns.flatMap((r) => r.events),
  };
}

function reset() {
  thinDraws.value = [];
  reschedDraws.value = [];
  thinStats.value = emptyStats();
  reschedStats.value = emptyStats();
  simRound.value = null;
  mode.value = "draw";
}

// Completed individuals count as samples even with zero events (f = 0).
const hasSamples = computed(() => {
  const t = thinStats.value;
  const r = reschedStats.value;
  return t.times.length + r.times.length + t.counts.length + r.counts.length > 0;
});
const canReset = computed(
  () => thinDraws.value.length > 0 || reschedDraws.value.length > 0 || hasSamples.value,
);

watch([curve, spec], reset);

const BLUE = "#2563eb";
const PURPLE = "#7c3aed";
const TEAL = "#0d9488";
const GRAY = "#6b7280";
const AMBER = "#d97706";
const TARGET = "#374151";
const axisTextStyle = { fontSize: 13 };

function fmt(v: number | null | undefined): string {
  return typeof v === "number" && Number.isFinite(v)
    ? (Math.round(v * 1000) / 1000).toString()
    : "—";
}
function fmtTau(v: unknown): string {
  const n = Number(v);
  return Number.isFinite(n)
    ? n.toLocaleString(undefined, { maximumFractionDigits: 2 })
    : "—";
}

const yMax = computed(() =>
  Math.max(0, ...curve.value.lambda, ...modified.value.lambda),
);
function activationSeries() {
  if (!active.value) return [];
  return [
    {
      x: [spec.value.activatesAt, spec.value.activatesAt],
      data: [0, yMax.value],
      color: AMBER,
      dashed: true,
      strokeWidth: 1.5,
      showInTooltip: false,
      legend: "modifier on",
    },
  ];
}
const hollow = {
  line: false,
  dots: true,
  color: GRAY,
  dotFill: "transparent",
  dotStroke: GRAY,
  showInTooltip: false,
};
const dotsOnly = (color: string) => ({
  color,
  line: false,
  dots: true,
  showInTooltip: false,
});

// Thinning: rejections sit on the forecast r(t); accepted attempts on f·r(t).
const thinSeries = computed(() => {
  const s: Record<string, unknown>[] = [
    {
      x: curve.value.x,
      data: curve.value.lambda,
      color: BLUE,
      strokeWidth: 2,
      legend: "r(t): forecast rate",
    },
    {
      x: modified.value.x,
      data: modified.value.lambda,
      color: PURPLE,
      strokeWidth: 2,
      legend: "accepted rate",
      showInTooltip: false,
    },
    ...activationSeries(),
  ];
  if (thinRejected.value.length) {
    s.push({
      ...hollow,
      x: thinRejected.value,
      data: thinRejected.value.map((t) => rateAtTime(curve.value, t)),
      legend: "rejected",
    });
  }
  if (thinEvents.value.length) {
    s.push({
      ...dotsOnly(PURPLE),
      x: thinEvents.value,
      data: thinEvents.value.map((t) => modifiedRateAt(curve.value, spec.value, t)),
      legend: "accepted",
    });
  }
  return s;
});
const modifiedArea = (color: string) => [
  {
    x: modified.value.x,
    upper: modified.value.lambda,
    lower: modified.value.lambda.map(() => 0),
    color,
    opacity: 0.18,
  },
];
const thinAreas = computed(() => modifiedArea(PURPLE));
const thinAnnotations = computed(() => {
  const a = thinAttempts.value[thinAttempts.value.length - 1];
  if (mode.value !== "draw" || !a || a.tau === null) return [];
  const text =
    a.outcome === "accepted"
      ? a.u === null
        ? "accept (p = 1)"
        : `accept: u = ${fmt(a.u)} < f`
      : `reject: u = ${fmt(a.u)} ≥ f`;
  const right = a.tau < curve.value.duration * 0.5;
  return [
    {
      x: a.tau,
      y: rateAtTime(curve.value, a.tau),
      text,
      offset: { x: right ? 10 : -10, y: 18 },
      align: (right ? "left" : "right") as "left" | "right",
      color: a.outcome === "accepted" ? PURPLE : GRAY,
      pointer: "none" as const,
      fontSize: 13,
    },
  ];
});

// Rescheduling: a cancelled forecast sits on r(t); events on the current rate.
const reschedSeries = computed(() => {
  const s: Record<string, unknown>[] = [
    {
      x: modified.value.x,
      data: modified.value.lambda,
      color: TEAL,
      strokeWidth: 2,
      legend: "current rate: r(t), then f·r(t)",
    },
    {
      x: curve.value.x,
      data: curve.value.lambda,
      color: BLUE,
      dashed: true,
      strokeWidth: 1.5,
      lineOpacity: 0.5,
      showInTooltip: false,
      legend: "r(t)",
    },
    ...activationSeries(),
  ];
  if (reschedCancelled.value.length) {
    s.push({
      ...hollow,
      x: reschedCancelled.value,
      data: reschedCancelled.value.map((t) => rateAtTime(curve.value, t)),
      legend: "cancelled",
    });
  }
  if (reschedEvents.value.length) {
    s.push({
      ...dotsOnly(TEAL),
      x: reschedEvents.value,
      data: reschedEvents.value.map((t) => modifiedRateAt(curve.value, spec.value, t)),
      legend: "event",
    });
  }
  return s;
});
const reschedAreas = computed(() => modifiedArea(TEAL));
const reschedAnnotations = computed(() => {
  const a = reschedAttempts.value[reschedAttempts.value.length - 1];
  if (mode.value !== "draw" || !a) return [];
  if (a.outcome === "cancelled" || a.outcome === "none") {
    const x = a.tau ?? spec.value.activatesAt;
    const right = x < curve.value.duration * 0.5;
    return [
      {
        x,
        y: rateAtTime(curve.value, x),
        text: a.tau === null ? "no forecast on r(t): drawn at tₘ" : "cancelled at tₘ, redrawn",
        offset: { x: right ? 10 : -10, y: 18 },
        align: (right ? "left" : "right") as "left" | "right",
        color: GRAY,
        pointer: "none" as const,
        fontSize: 13,
      },
    ];
  }
  if (a.tau === null) return [];
  const right = a.tau < curve.value.duration * 0.5;
  return [
    {
      x: a.tau,
      y: modifiedRateAt(curve.value, spec.value, a.tau),
      text: a.rescheduled
        ? "redrawn at tₘ: event on f·r(t)"
        : a.onModified
          ? "event on f·r(t)"
          : "event on r(t)",
      offset: { x: right ? 10 : -10, y: 18 },
      align: (right ? "left" : "right") as "left" | "right",
      color: TEAL,
      pointer: "none" as const,
      fontSize: 13,
    },
  ];
});

// Pooled event-time distributions + observed R₀.
// Capped below the Explore tab's 60 because two series share each bin.
const MAX_BINS = 40;
const distBins = computed(() => {
  const n = thinningOk.value
    ? Math.min(thinStats.value.times.length, reschedStats.value.times.length)
    : reschedStats.value.times.length;
  return histogramBins(n, 8, MAX_BINS);
});
const thinHist = computed(() =>
  eventTimeHistogram(thinStats.value.times, curve.value.duration, distBins.value),
);
const reschedHist = computed(() =>
  eventTimeHistogram(reschedStats.value.times, curve.value.duration, distBins.value),
);
const distSeries = computed(() => [
  ...(thinningOk.value
    ? [{ data: thinHist.value.percentages, color: PURPLE, legend: "Thinning" }]
    : []),
  { data: reschedHist.value.percentages, color: TEAL, legend: "Rescheduling" },
]);
// Sampled densely in category-index space (bin i is centered at i) so the
// step at tₘ stays sharp.
const distOverlay = computed(() => {
  const { binWidth, centers } = reschedHist.value;
  if (!centers.length) return [];
  const N = 200;
  const x: number[] = [];
  const data: number[] = [];
  for (let i = 0; i < N; i++) {
    const t = (curve.value.duration * i) / (N - 1);
    x.push(t / binWidth - 0.5);
    data.push(modifiedRateAt(curve.value, spec.value, t));
  }
  return [
    { x, data, color: TARGET, strokeWidth: 2, dots: false, legend: "f·r(t)" },
  ];
});
function distLabel(label: string, index: number): string {
  return sparseBinLabel(label, index, distBins.value);
}
function observedR0(s: MethodStats): number | null {
  if (!s.counts.length) return null;
  return s.counts.reduce((a, b) => a + b, 0) / s.counts.length;
}
function perIndividual(s: MethodStats): number | null {
  if (!s.counts.length) return null;
  return s.discarded / s.counts.length;
}
const summaryRows = computed(() => [
  ...(thinningOk.value
    ? [
        {
          method: "Thinning",
          color: PURPLE,
          stats: thinStats.value,
          discardedLabel: "rejected attempts",
        },
      ]
    : []),
  {
    method: "Rescheduling",
    color: TEAL,
    stats: reschedStats.value,
    discardedLabel: "cancelled forecasts",
  },
]);

const thinTimelineSim = computed(() =>
  mode.value === "sim" ? (simRound.value?.thin ?? []) : null,
);
const reschedTimelineSim = computed(() =>
  mode.value === "sim" ? (simRound.value?.resched ?? []) : null,
);
const timelinesVisible = computed(
  () => mode.value === "sim" || thinDraws.value.length > 0 || reschedDraws.value.length > 0,
);
</script>

<template>
  <section class="mexp">
    <header class="mexp-header">
      <h2>Transmission modifiers</h2>
      <p class="mexp-subtitle">{{ desc.title }}</p>
      <p class="mexp-hint">
        A modifier switches on partway through an infection and scales the person's intrinsic rate r(t) by a
        factor <strong>f</strong> from then on. Below 1 it reduces infectiousness (an antiviral: 1 − efficacy after
        its delay; a facemask: 1 − effectiveness once donned); above 1 it raises it. The model already forecast that person's next attempt on the
        unmodified r(t), so we need to reconcile the pending forecast with the new rate. There are two ways to do this: thinning and rescheduling.
      </p>
      <div class="mexp-controls">
        <NumberInput v-model="activatesAt" label="Activates at (day)" :min="0" :max="curve.duration" :step="0.1"
          slider live />
        <NumberInput v-model="factor" label="Infectiousness multiplier f" :min="0" :max="2" :step="0.05" slider live
          hint="r(t) is multiplied by f once the modifier is on: 1 − effectiveness (facemask) or 1 − efficacy (antiviral) for a reduction; above 1 for a modifier that raises infectiousness." />
      </div>
      <p v-if="!active" class="mexp-note">
        The modifier is inert for this rate: it never activates, or leaves the rate unchanged.
      </p>
      <div class="mexp-buttons">
        <Button :disabled="!hasArea" @click="drawNext">Draw next</Button>
        <Button variant="secondary" :disabled="!hasArea" @click="simulateMany">Simulate {{ SIM_RUNS }}×</Button>
        <Button variant="secondary" :disabled="!canReset" @click="reset">Reset</Button>
      </div>
    </header>

    <div class="mexp-grid">
      <!-- Thinning -->
      <div class="mexp-panel mexp-panel-thin" :class="{ 'mexp-panel-off': !thinningOk }">
        <h3>Thinning (rejection sampling)</h3>
        <p v-if="!thinningOk" class="mexp-note mexp-off-note">
          Not available when the modifier raises the rate (f = {{ fmt(factor) }} &gt; 1): the unmodified r(t) is
          no longer an upper bound, so current ÷ forecast would exceed 1 and there is nothing to thin. Only
          rescheduling can be used.
        </p>
        <div class="mexp-panel-body">
        <p class="mexp-hint">
          Every attempt is forecast on the unmodified <span class="c-blue">r(t)</span>, an upper bound on the
          true rate. When the attempt time arrives, accept it with probability <strong>current ÷ forecast</strong>.
          Before the modifier every attempt is accepted; after, it is f·r(τ) ÷ r(τ) = <strong>f</strong>. A rejected attempt is discarded but still used
          to calculate the next attempt time.
        </p>
        <LineChart :series="thinSeries" :areas="thinAreas" :annotations="thinAnnotations" :chart-padding="{ top: 16 }"
          :height="230" :y-min="0" :menu="false" :tick-label-style="axisTextStyle" :axis-label-style="axisTextStyle"
          :legend-style="axisTextStyle" x-label="t (days since infected)" y-label="rate" tooltip-trigger="hover">
          <template #tooltip="{ xLabel, values }">
            <div class="mexp-tooltip">
              <div v-if="xLabel != null">Day {{ fmtTau(xLabel) }}</div>
              <div>r(t) = {{ fmtTau(values[0]?.value) }}</div>
            </div>
          </template>
        </LineChart>
        <ModifierTimeline v-show="timelinesVisible" :duration="curve.duration" :activates-at="spec.activatesAt"
          :events="thinEvents" :discarded="thinRejected" :sim-times="thinTimelineSim" :recovered="thinExhausted"
          :color="PURPLE" discarded-label="rejected attempt" class="mexp-timeline mexp-timeline-thin" />
        <table v-if="mode === 'draw' && thinAttempts.length" class="mexp-table">
          <thead>
            <tr>
              <th>e ~ Exp(1)</th>
              <th>
                τ
                <Hint text="Forecast attempt time on the unmodified r(t): invert its cumulative rate at the running total of the e's." />
              </th>
              <th>
                p
                <Hint text="Acceptance probability = current ÷ forecast rate at τ: 1 before the modifier is on, f after." />
              </th>
              <th>u ~ U(0,1)</th>
              <th>outcome</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(a, i) in thinAttempts" :key="i">
              <td class="num">{{ fmt(a.e) }}</td>
              <td class="num">{{ a.tau === null ? "—" : fmt(a.tau) }}</td>
              <td class="num">{{ a.tau === null ? "—" : fmt(a.pAccept) }}</td>
              <td class="num">{{ a.u === null ? "—" : fmt(a.u) }}</td>
              <td>
                <span v-if="a.outcome === 'accepted'" class="c-purple">accepted</span>
                <span v-else-if="a.outcome === 'rejected'" class="c-gray">rejected</span>
                <span v-else class="muted">recovered</span>
              </td>
            </tr>
          </tbody>
        </table>
        </div>
      </div>

      <!-- Rescheduling -->
      <div class="mexp-panel mexp-panel-resched">
        <h3>Rescheduling</h3>
        <p class="mexp-hint">
          Every attempt is forecast on the <span class="c-teal">current</span> rate. The moment the modifier
          switches on at <strong>tₘ</strong>, a new event is scheduled
          from tₘ on <strong>f·r(t)</strong>. The original event must be <strong>cancelled</strong>.
        </p>
        <LineChart :series="reschedSeries" :areas="reschedAreas" :annotations="reschedAnnotations"
          :chart-padding="{ top: 16 }" :height="230" :y-min="0" :menu="false" :tick-label-style="axisTextStyle"
          :axis-label-style="axisTextStyle" :legend-style="axisTextStyle" x-label="t (days since infected)"
          y-label="rate" tooltip-trigger="hover">
          <template #tooltip="{ xLabel, values }">
            <div class="mexp-tooltip">
              <div v-if="xLabel != null">Day {{ fmtTau(xLabel) }}</div>
              <div>current rate = {{ fmtTau(values[0]?.value) }}</div>
            </div>
          </template>
        </LineChart>
        <ModifierTimeline v-show="timelinesVisible" :duration="curve.duration" :activates-at="spec.activatesAt"
          :events="reschedEvents" :discarded="reschedCancelled" :sim-times="reschedTimelineSim"
          :recovered="reschedExhausted" :color="TEAL" discarded-label="cancelled forecast"
          class="mexp-timeline mexp-timeline-resched" />
        <table v-if="mode === 'draw' && reschedAttempts.length" class="mexp-table">
          <thead>
            <tr>
              <th>e ~ Exp(1)</th>
              <th>
                on
                <Hint text="Which rate the forecast is inverted on: r(t) before the modifier, f·r(t) after." />
              </th>
              <th>
                from
                <Hint text="Where the forecast starts: the previous event, or tₘ for the redraw made when the modifier activates." />
              </th>
              <th>τ</th>
              <th>outcome</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(a, i) in reschedAttempts" :key="i">
              <td class="num">{{ fmt(a.e) }}</td>
              <td>{{ a.onModified ? "f·r(t)" : "r(t)" }}</td>
              <td class="num">{{ fmt(a.from) }}</td>
              <td class="num">{{ a.tau === null ? "—" : fmt(a.tau) }}</td>
              <td>
                <span v-if="a.outcome === 'event'" class="c-teal">event</span>
                <span v-else-if="a.outcome === 'cancelled'" class="c-gray">cancelled at tₘ</span>
                <span v-else-if="a.outcome === 'none'" class="c-gray">no forecast on r(t)</span>
                <span v-else class="muted">recovered</span>
                <span v-if="a.rescheduled" class="muted"> · drawn at tₘ</span>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <p v-if="!hasSamples && !thinDraws.length && !reschedDraws.length" class="mexp-hint mexp-empty">
      No samples yet. Press <strong>Draw next</strong> to forecast one attempt{{ thinningOk ? " under each method" : "" }}, or
      <strong>Simulate {{ SIM_RUNS }}×</strong> to build the distribution{{ thinningOk ? "s" : "" }} at once.
    </p>

    <div class="mexp-equiv">
      <h3>Distribution of simulated event times</h3>
      <p class="mexp-hint">
        Event times pooled across every round, binned over the infectious period{{ thinningOk ? ", for each method. Both approach" : ". They approach" }}
        the modified rate <strong>f·r(t)</strong>; expected infections per individual = the shaded area
        <strong>{{ fmt(modified.total) }}</strong>.
      </p>
      <template v-if="hasSamples">
        <BarChart :categories="reschedHist.categories" :series="distSeries" :summary-lines="distOverlay" :height="220"
          :menu="false" x-label="t (days since infected)" y-label="% of that method's events"
          :tick-label-style="axisTextStyle" :axis-label-style="axisTextStyle" :legend-style="axisTextStyle"
          value-tick-format="%.0f%%" :category-format="distLabel">
          <template #tooltip="{ category, values }">
            <div class="mexp-tooltip">
              <div>t ≈ {{ category }}</div>
              <div v-if="thinningOk" class="c-purple">thinning: {{ fmtTau(values[0]?.value) }}%</div>
              <div class="c-teal">rescheduling: {{ fmtTau(values[thinningOk ? 1 : 0]?.value) }}%</div>
            </div>
          </template>
        </BarChart>
        <table class="mexp-table mexp-summary">
          <thead>
            <tr>
              <th>Method</th>
              <th>Individuals</th>
              <th>Events</th>
              <th>
                Observed R₀
                <Hint :text="`Mean events per completed individual; expected ${fmt(modified.total)}.`" />
              </th>
              <th>Discarded forecasts</th>
              <th>Per individual</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in summaryRows" :key="row.method">
              <td><span class="swatch" :style="{ background: row.color }" /> {{ row.method }}</td>
              <td class="num">{{ row.stats.counts.length }}</td>
              <td class="num">{{ row.stats.times.length }}</td>
              <td class="num">{{ fmt(observedR0(row.stats)) }}</td>
              <td class="num">{{ row.stats.discarded }} {{ row.discardedLabel }}</td>
              <td class="num">{{ fmt(perIndividual(row.stats)) }}</td>
            </tr>
          </tbody>
        </table>
      </template>
    </div>
  </section>
</template>

<style scoped>
.mexp {
  display: flex;
  flex-direction: column;
  gap: 1em;
}

.mexp-header {
  display: flex;
  flex-direction: column;
  gap: 0.4em;
}

.mexp-header h2 {
  margin: 0;
}

.mexp-subtitle {
  margin: 0;
  color: var(--color-text-secondary);
}

.mexp-hint {
  margin: 0;
  font-size: var(--font-size-sm, 0.875rem);
  color: var(--color-text-secondary);
}

.mexp-hint strong {
  color: var(--color-text);
  font-variant-numeric: tabular-nums;
}

.mexp-note {
  margin: 0;
  font-size: var(--font-size-sm, 0.875rem);
  color: var(--color-text-secondary);
  font-style: italic;
}

.mexp-controls {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.5em 1.5em;
  margin-top: 0.4em;
}

.mexp-buttons {
  display: flex;
  gap: 0.4em;
  margin-top: 0.4em;
}

.mexp-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 1.5em;
}

@media (max-width: 900px) {
  .mexp-grid,
  .mexp-controls {
    grid-template-columns: 1fr;
  }
}

.mexp-panel {
  display: flex;
  flex-direction: column;
  gap: 0.4em;
  min-width: 0;
}

.mexp-panel h3,
.mexp-equiv h3 {
  margin: 0;
  font-size: var(--font-size-md, 1rem);
}

.mexp-timeline {
  margin-top: 0.4em;
}

.mexp-panel-body {
  display: flex;
  flex-direction: column;
  gap: 0.4em;
}

/* Greyed out, not removed, so the chart still shows f·r(t) above the bound. */
.mexp-panel-off .mexp-panel-body {
  opacity: 0.4;
  pointer-events: none;
  user-select: none;
}

.mexp-off-note {
  font-style: normal;
  color: var(--color-text);
  padding: 0.4em 0.6em;
  border-left: 3px solid #d97706;
  background: color-mix(in srgb, #d97706 8%, transparent);
}

.mexp-empty {
  margin-top: 0.2em;
}

.mexp-equiv {
  display: flex;
  flex-direction: column;
  gap: 0.5em;
  padding-top: 0.8em;
  border-top: 1px solid var(--color-border);
}

.mexp-table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--font-size-sm, 0.875rem);
  font-variant-numeric: tabular-nums;
}

.mexp-table th,
.mexp-table td {
  padding: 0.25em 0.5em;
  text-align: left;
  border-bottom: 1px solid var(--color-border);
}

.mexp-table th {
  color: var(--color-text-secondary);
  font-weight: 600;
}

.swatch {
  display: inline-block;
  width: 0.7em;
  height: 0.7em;
  border-radius: 2px;
  margin-right: 0.3em;
  vertical-align: baseline;
}

.c-blue {
  color: #2563eb;
  font-weight: 600;
}

.c-purple {
  color: #7c3aed;
  font-weight: 600;
}

.c-teal {
  color: #0d9488;
  font-weight: 600;
}

.c-gray {
  color: #6b7280;
  font-weight: 600;
}

.muted {
  color: var(--color-text-secondary);
  font-style: italic;
}

.mexp-tooltip {
  display: flex;
  flex-direction: column;
  gap: 1px;
  font-size: 0.8125rem;
  white-space: nowrap;
}
</style>
