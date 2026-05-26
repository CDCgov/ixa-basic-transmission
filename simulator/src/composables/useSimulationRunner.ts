import { ref, watch, onScopeDispose, computed, type Ref } from "vue";
import { runWasm, cancelWasm } from "@cfasim-ui/wasm";
import { ModelOutput } from "@cfasim-ui/shared";
import type { ColumnDescriptor, TypedColumn } from "@cfasim-ui/shared";

// Per-batch target workload (population × sims-in-batch). Roughly chosen
// so each batch finishes in ~100–200ms on a default-ish machine. Smaller
// batches → faster progressive updates but more postMessage overhead.
const BATCH_TARGET_WORKLOAD = 10_000;

interface BatchArgs {
  population: number;
  nSimulations: number;
}

interface Accumulator {
  time: Float64Array;
  trajectories: TypedColumn[];
  arPerRun: number[];
}

function batchSizeFor(p: BatchArgs): number {
  const perBatch = Math.max(
    1,
    Math.floor(BATCH_TARGET_WORKLOAD / p.population),
  );
  return Math.min(perBatch, p.nSimulations);
}

// Pointwise median across an ensemble of equal-length trajectories.
// Mirrors `stats::pointwise_median` in `src/stats.rs`.
function pointwiseMedian(trajectories: TypedColumn[]): Float64Array {
  if (trajectories.length === 0) return new Float64Array(0);
  const len = trajectories[0].length;
  const out = new Float64Array(len);
  const buf: number[] = [];
  for (let i = 0; i < len; i++) {
    buf.length = 0;
    for (const t of trajectories) {
      const v = t[i];
      if (Number.isFinite(v)) buf.push(v);
    }
    buf.sort((a, b) => a - b);
    const n = buf.length;
    if (n === 0) out[i] = NaN;
    else if (n % 2 === 1) out[i] = buf[(n - 1) / 2];
    else out[i] = (buf[n / 2 - 1] + buf[n / 2]) / 2;
  }
  return out;
}

// Linear-interpolation median; mirrors `stats::median` in src/stats.rs.
function median1D(values: number[]): number {
  const finite = values.filter(Number.isFinite).sort((a, b) => a - b);
  const n = finite.length;
  if (n === 0) return NaN;
  return n % 2 === 1
    ? finite[(n - 1) / 2]
    : (finite[n / 2 - 1] + finite[n / 2]) / 2;
}

function f64Col(name: string): ColumnDescriptor {
  return { name, type: "f64" };
}

function assembleOutputs(acc: Accumulator): Record<string, ModelOutput> {
  const median = pointwiseMedian(acc.trajectories);
  const seriesColumns: ColumnDescriptor[] = [
    f64Col("time"),
    ...acc.trajectories.map((_, i) => f64Col(`cumulative_infections_${i}`)),
    f64Col("cumulative_infections_median"),
  ];
  const seriesBuffers: TypedColumn[] = [
    acc.time,
    ...acc.trajectories,
    median,
  ];
  const series = new ModelOutput(
    acc.time.length,
    seriesColumns,
    seriesBuffers,
  );

  const summaryColumns: ColumnDescriptor[] = [
    f64Col("attack_rate_observed_median"),
  ];
  const summaryBuffers: TypedColumn[] = [
    Float64Array.of(median1D(acc.arPerRun)),
  ];
  const summary = new ModelOutput(1, summaryColumns, summaryBuffers);

  return { series, summary };
}

export interface SimulationRunner {
  outputs: Ref<Record<string, ModelOutput> | undefined>;
  error: Ref<string | undefined>;
  loading: Ref<boolean>;
  statusMessage: Ref<string | null>;
}

/// Single-slot coalescing scheduler with progressive batching. While a run
/// is in flight, new param changes overwrite the pending slot rather than
/// queueing. Each run streams `simulate_batch` calls one at a time,
/// accumulating trajectories and re-assembling `outputs.value` after every
/// batch so the chart fills in progressively. Cancellation is implicit:
/// when `currentRunId` changes, the in-flight `drain` notices between
/// batches and aborts — no worker terminate needed, so the wasm module
/// cache survives.
export function useSimulationRunner<P extends BatchArgs>(
  params: P,
  options: { wasmName: string },
): SimulationRunner {
  const outputs = ref<Record<string, ModelOutput>>();
  const error = ref<string>();
  const loading = ref(false);
  const progress = ref<{ done: number; total: number } | null>(null);
  // Sticks around between runs so the status line can report the last
  // natural (non-cancelled) completion as e.g. "Ran 20 simulations".
  const lastRunSimCount = ref<number | null>(null);

  let currentRunId = 0;
  let pendingArgs: string | null = null;

  async function drain() {
    if (pendingArgs === null) return;
    const argsJson = pendingArgs;
    pendingArgs = null;
    const myId = ++currentRunId;
    const p = JSON.parse(argsJson) as P;
    const batchSize = batchSizeFor(p);
    loading.value = true;
    error.value = undefined;
    progress.value = null;
    console.log(
      `[ixa] run #${myId} started (workload=${(
        p.population * p.nSimulations
      ).toLocaleString()}, batchSize=${batchSize})`,
    );

    let accumulator: Accumulator | null = null;
    let done = 0;
    try {
      while (done < p.nSimulations) {
        const thisBatch = Math.min(batchSize, p.nSimulations - done);
        const batchArgs = JSON.stringify({
          ...p,
          batchSize: thisBatch,
          seedOffset: done,
        });
        const res = (await runWasm(
          options.wasmName,
          "simulate_batch",
          batchArgs,
        )) as Record<string, ModelOutput>;
        if (myId !== currentRunId) {
          console.log(
            `[ixa] run #${myId} cancelled (${done}/${p.nSimulations} sims completed)`,
          );
          return;
        }
        const batchSeries = res.series;
        const perRun = res.per_run;
        if (accumulator === null) {
          accumulator = {
            time: batchSeries.column("time") as Float64Array,
            trajectories: [],
            arPerRun: [],
          };
        }
        for (let i = 0; i < thisBatch; i++) {
          accumulator.trajectories.push(
            batchSeries.column(`cumulative_infections_${i}`),
          );
        }
        const arCol = perRun.column("attack_rate_per_run");
        for (let i = 0; i < thisBatch; i++) {
          accumulator.arPerRun.push(arCol[i]);
        }
        done += thisBatch;
        outputs.value = assembleOutputs(accumulator);
        progress.value = { done, total: p.nSimulations };
        console.log(
          `[ixa] run #${myId} progress: ${done}/${p.nSimulations} sims`,
        );
      }
      // Natural completion (any `return` from cancellation already exited
      // before this line). Stamp the count so the status line can read it.
      lastRunSimCount.value = done;
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (myId === currentRunId && msg !== "cancelled") {
        error.value = msg;
      }
    } finally {
      if (myId === currentRunId) {
        loading.value = false;
        progress.value = null;
      }
      if (pendingArgs !== null) drain();
    }
  }

  function scheduleRun(argsJson: string) {
    pendingArgs = argsJson;
    if (!loading.value) {
      drain();
    } else {
      // Bump so the in-flight drain notices between batches (its
      // `myId !== currentRunId` check) and bails out. Its `finally`
      // then re-enters `drain` for the pending args.
      currentRunId++;
    }
  }

  watch(() => JSON.stringify(params), scheduleRun, { immediate: true });

  onScopeDispose(() => {
    // Drop any queued args first so `drain`'s finally doesn't re-enter on
    // a dead component after `cancelWasm` rejects the in-flight runWasm.
    pendingArgs = null;
    if (loading.value) cancelWasm();
  });

  const statusMessage = computed(() => {
    if (loading.value) {
      const p = progress.value;
      return p
        ? `Running simulation… (${p.done}/${p.total} sims)`
        : "Running simulation…";
    }
    if (lastRunSimCount.value !== null) {
      return `Ran ${lastRunSimCount.value} simulations`;
    }
    return null;
  });

  return { outputs, error, loading, statusMessage };
}
