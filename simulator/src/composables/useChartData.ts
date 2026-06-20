import { computed, type Ref } from "vue";
import { ModelOutput } from "cfasim-ui/shared";
import type { TypedColumn } from "cfasim-ui/shared";
import type { ChartAnnotation } from "cfasim-ui/charts";
import { expectedR0, type InfectionRate } from "./infectionRate";
import type { SettingType } from "./settings";

// Chart layers, in render order:
//   1. The N stochastic trajectories as a translucent blue "fan" (no legend).
//   2. The pointwise median across the fan (computed in JS via
//      `pointwiseMedian` inside `assembleOutputs`, exposed as
//      `cumulative_infections_median`).
const isFanColumn = (n: string) => /^cumulative_infections_\d+$/.test(n);

// Incidence = first differences of the cumulative curves. The cumulative
// columns are forward-filled at integer time bins, so incidence[t] is the
// number of new infections during the interval (t-1, t]. incidence[0] is
// defined as 0 to keep array lengths aligned with the time axis.
function diff(arr: ArrayLike<number>): number[] {
  const out = new Array(arr.length);
  out[0] = 0;
  for (let i = 1; i < arr.length; i++) out[i] = arr[i] - arr[i - 1];
  return out;
}

function argmax(values: ArrayLike<number>): number {
  let bestI = -1;
  let bestV = -Infinity;
  for (let i = 0; i < values.length; i++) {
    const v = values[i];
    if (Number.isFinite(v) && v > bestV) {
      bestV = v;
      bestI = i;
    }
  }
  return bestI;
}

function fmtCount(v: number | undefined): string {
  return v != null && Number.isFinite(v)
    ? Math.round(v).toLocaleString()
    : "—";
}

export interface Chart {
  series: ReturnType<typeof buildSeries>;
  annotations: ChartAnnotation[];
  padding?: { top?: number };
  yLabel: string;
  height: number;
}

function buildSeries(
  outputs: Record<string, ModelOutput> | undefined,
  transform: (col: TypedColumn) => TypedColumn | number[],
) {
  const s = outputs?.series;
  if (!s) return [];
  // Filtering by column name handles re-runs where nSimulations has
  // changed and the wasm output has more/fewer trajectories than before.
  const fan = s.names.filter(isFanColumn).map((n) => ({
    data: transform(s.column(n)),
    color: "#2563eb",
    opacity: 0.2,
  }));
  return [
    ...fan,
    {
      data: transform(s.column("cumulative_infections_median")),
      color: "#f87171",
      strokeWidth: 2,
      legend: "Median observed",
    },
  ];
}

export function useChartData(
  outputs: Ref<Record<string, ModelOutput> | undefined>,
  infectionRate: Ref<InfectionRate>,
  settings: Ref<SettingType[]>,
) {
  const incidenceAnnotations = computed<ChartAnnotation[]>(() => {
    const s = outputs.value?.series;
    if (!s) return [];
    const med = diff(s.column("cumulative_infections_median"));
    const i = argmax(med);
    if (i < 0) return [];
    return [
      {
        x: i,
        y: med[i],
        text: `**Peak**\nt = ${i}, ${fmtCount(med[i])}`,
        offset: { x: 24, y: -28 },
        color: "#f87171",
      },
    ];
  });

  const charts = computed<Chart[]>(() => [
    {
      series: buildSeries(outputs.value, (c) => c),
      annotations: [],
      padding: undefined,
      yLabel: "Cumulative infections",
      height: 400,
    },
    {
      series: buildSeries(outputs.value, diff),
      annotations: incidenceAnnotations.value,
      // Top padding gives the 2-line "Peak" label room between the
      // inline legend (now pinned to the top) and the curve.
      padding: { top: 40 },
      yLabel: "Incidence",
      height: 300,
    },
  ]);

  // Summary table: observed median attack rate (from the ensemble) plus
  // the expected R₀ from `composables/infectionRate#expectedR0` — the
  // random-mixing R₀ adjusted for the current settings configuration.
  const summary = computed(() => {
    const s = outputs.value?.summary;
    if (!s) return null;
    const arMedian = s.column("attack_rate_observed_median")[0];
    const fmtR0 = (v: number) => (Number.isFinite(v) ? v.toFixed(2) : "—");
    const fmtAr = (v: number) =>
      Number.isFinite(v) ? `${(v * 100).toFixed(1)}%` : "—";
    return {
      metric: ["R₀ (expected)", "Attack rate (observed median)"],
      value: [
        fmtR0(expectedR0(infectionRate.value, settings.value)),
        fmtAr(arMedian),
      ],
    };
  });

  return { charts, summary, fmtCount };
}
