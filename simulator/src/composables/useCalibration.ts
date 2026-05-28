// Streaming + persistent ABC-SMC scheduler. Mirrors the single-slot
// pattern in `useSimulationRunner.ts` but writes to IndexedDB after each
// batch so the run can be paused, the tab closed, and resumed later.
//
// JS owns the stage loop; wasm only knows how to produce one batch.
// See `src/lib.rs::calibrate_batch` for the wasm contract.

import { ref, onScopeDispose, type Ref } from "vue";
import { runWasm } from "cfasim-ui/wasm";
import { ModelOutput } from "cfasim-ui/shared";
import {
  computeThreshold,
  type CalibrationConfig,
  type Particle,
} from "./calibration";
import {
  loadRun,
  saveBatch,
  setStatus,
  type RunStatus,
  type StoredRun,
} from "./calibrationStorage";

const WASM_NAME = "ixa_basic_transmission";

export interface CalibrationRunner {
  status: Ref<RunStatus>;
  runId: Ref<string | null>;
  runName: Ref<string>;
  config: Ref<CalibrationConfig | null>;
  currentStage: Ref<number>;
  // Index by stage (stage 0 = prior sample, stage k≥1 = perturbation
  // from previous). Stages start as empty arrays; particles stream in.
  particlesByStage: Ref<Particle[][]>;
  acceptance: Ref<{ nAttempts: number; nAccepted: number }[]>;
  errorMessage: Ref<string | undefined>;
  /// Load a run from IDB into the refs without starting the loop. Use
  /// this for an idle/complete run that the user might want to inspect
  /// or kick off later.
  select: (runId: string) => Promise<void>;
  /// Kick off (or resume) the loop for whatever run is currently
  /// loaded. No-op if nothing is selected, already running, or complete.
  run: () => Promise<void>;
  pause: () => void;
  reset: () => void;
}

export function useCalibration(): CalibrationRunner {
  const status = ref<RunStatus>("idle");
  const runId = ref<string | null>(null);
  const runName = ref<string>("");
  const config = ref<CalibrationConfig | null>(null);
  const currentStage = ref<number>(0);
  const particlesByStage = ref<Particle[][]>([]);
  const acceptance = ref<{ nAttempts: number; nAccepted: number }[]>([]);
  const errorMessage = ref<string | undefined>(undefined);

  // Set when pause() is called; the loop checks between batches.
  let pauseRequested = false;
  // Bumped on reset() so a stale in-flight loop notices and bails.
  // `runOneBatch` also reads this *after* its await so a batch that was
  // already in the worker when reset() fired doesn't resurrect the
  // refs/IDB rows that reset() just cleared.
  let currentLoopId = 0;

  function isCurrent(loopId: number): boolean {
    return loopId === currentLoopId && !pauseRequested;
  }

  function reset() {
    pauseRequested = true;
    currentLoopId++;
    status.value = "idle";
    runId.value = null;
    runName.value = "";
    config.value = null;
    currentStage.value = 0;
    particlesByStage.value = [];
    acceptance.value = [];
    errorMessage.value = undefined;
  }

  function pause() {
    if (status.value === "running") {
      pauseRequested = true;
    }
  }

  // Unmount-safety: a component navigating away mid-run shouldn't leave
  // the loop spinning. `reset()` flips the cancellation flag; the
  // in-flight loop bails on its next post-await check.
  onScopeDispose(() => {
    pauseRequested = true;
    currentLoopId++;
  });

  async function select(id: string): Promise<void> {
    const loaded = await loadRun(id);
    if (!loaded) {
      throw new Error(`Run ${id} not found`);
    }
    runId.value = id;
    runName.value = loaded.run.name;
    config.value = loaded.run.config;
    particlesByStage.value = loaded.particlesByStage;
    acceptance.value = [...loaded.run.acceptance];
    currentStage.value = loaded.run.currentStage;
    errorMessage.value = loaded.run.errorMessage;
    // A persisted "running" status is always stale: the loop only ever
    // lives in memory, so if we're loading from IDB the page was
    // refreshed/reopened and no loop is actually running. Present it as
    // "paused" so the UI offers Resume (and `run()` doesn't no-op on it).
    status.value = loaded.run.status === "running" ? "paused" : loaded.run.status;
  }

  /// Enter the batch loop for the currently-selected run. Idle/paused/
  /// error runs become "running" and fire batches; complete runs are
  /// a no-op (nothing to do); running runs would race the in-flight
  /// loop, so we no-op them too.
  async function run(): Promise<void> {
    const id = runId.value;
    if (!id) return;
    if (status.value === "complete" || status.value === "running") return;
    // Re-fetch from IDB so we pick up any updateConfig that happened
    // between select() and run() (the sidebar edits flow that lets the
    // user tweak priors on a freshly-created idle run before starting).
    const loaded = await loadRun(id);
    if (!loaded) {
      throw new Error(`Run ${id} not found`);
    }
    config.value = loaded.run.config;
    pauseRequested = false;
    await runLoop(loaded.run);
  }

  async function runLoop(run: StoredRun): Promise<void> {
    const myId = ++currentLoopId;
    status.value = "running";
    // Explicit `null` (not `undefined`) so the storage layer treats it
    // as "clear this field" — `undefined` is dropped by JSON.stringify
    // and leaves a stale errorMessage on a previously-errored run.
    await setStatus(run.id, { status: "running", errorMessage: null });
    try {
      const c = run.config;
      // +1 because stage 0 is the implicit prior-sample pass; stages[k]
      // refers to the threshold for stage k+1.
      const totalStages = c.stages.length + 1;
      while (true) {
        if (!isCurrent(myId)) {
          status.value = "paused";
          await setStatus(run.id, {
            status: "paused",
            currentStage: currentStage.value,
            acceptance: acceptance.value,
          });
          return;
        }
        const stage = currentStage.value;
        if (stage >= totalStages) {
          status.value = "complete";
          await setStatus(run.id, {
            status: "complete",
            currentStage: stage,
            acceptance: acceptance.value,
          });
          return;
        }
        const have = particlesByStage.value[stage]?.length ?? 0;
        if (have < c.nParticles) {
          await runOneBatch(run.id, c, stage, have, myId);
        } else {
          // Stage filled — advance.
          currentStage.value = stage + 1;
          await setStatus(run.id, {
            currentStage: currentStage.value,
            acceptance: acceptance.value,
          });
        }
      }
    } catch (e) {
      // If this loop was already cancelled when the throw happened,
      // suppress the error — `runOneBatch` may legitimately reject when
      // the user discards or resets mid-batch.
      if (myId !== currentLoopId) return;
      const msg = e instanceof Error ? e.message : String(e);
      errorMessage.value = msg;
      status.value = "error";
      await setStatus(run.id, { status: "error", errorMessage: msg });
    }
  }

  async function runOneBatch(
    id: string,
    c: CalibrationConfig,
    stage: number,
    haveSoFar: number,
    loopId: number,
  ): Promise<void> {
    // Defensive: a 0 batch would let the outer loop spin forever waiting
    // for particles that never arrive. The wasm side also asserts this.
    const batchSize = Math.max(1, Math.min(c.batchSize, c.nParticles - haveSoFar));
    // Stage 0: sample from prior at ∞ (null = ∞ on wasm; JSON.stringify
    // turns Infinity into "null" anyway, so make the convention explicit).
    // Stage 1: rejection-sample from prior at threshold = quantile of
    // stage 0's distances. Stage 2+: perturb from previous stage. All
    // post-stage-0 thresholds are quantiles anchored to stage 0, so the
    // schedule doesn't compound across stages.
    let threshold: number | null = null;
    let previousParticles: Particle[] = [];
    if (stage > 0) {
      const reference = particlesByStage.value[0] ?? [];
      threshold = computeThreshold(reference, c.stages[stage - 1]);
      if (stage > 1) {
        previousParticles = particlesByStage.value[stage - 1] ?? [];
      }
    }
    const args = {
      infectionRate: c.modelContext.infectionRate,
      population: c.modelContext.population,
      maxTime: c.modelContext.maxTime,
      settings: c.modelContext.settings,
      observed: c.observed,
      // Gap-aware distance: compare only at these 1-based days (empty →
      // every day). See `data_distance` in Rust.
      observedDays: c.observedDays ?? [],
      // true = cumulative-incidence distance, false = per-day (daily).
      cumulative: (c.distanceMetric ?? "cumulative") !== "daily",
      initialInfectionsLo: c.priors.initialInfectionsLo,
      initialInfectionsHi: c.priors.initialInfectionsHi,
      r0Lo: c.priors.r0Lo,
      r0Hi: c.priors.r0Hi,
      stage,
      errorThreshold: threshold,
      previousParticles: previousParticles.map((p) => ({
        r0: p.r0,
        initialInfections: p.initialInfections,
        weight: p.weight,
      })),
      // Undefined for v2 runs → wasm side defaults to 2.0.
      varianceFactor: c.varianceFactor,
      // Undefined → wasm side defaults to 0.0 (always replace seed).
      probKeepSeed: c.probKeepSeed,
      batchSize,
      particleOffset: haveSoFar,
      seed: c.seed,
    };
    const res = (await runWasm(
      WASM_NAME,
      "calibrate_batch",
      JSON.stringify(args),
    )) as Record<string, ModelOutput>;
    // Cancellation gate: if reset()/discard fired while wasm was busy,
    // drop the result on the floor. Without this, the resolving promise
    // would write particles back into the cleared refs and re-insert
    // rows into a just-deleted IDB run.
    if (!isCurrent(loopId)) return;
    const particles = res.particles;
    const r0Col = particles.column("r0");
    const iiCol = particles.column("initial_infections");
    const wCol = particles.column("weight");
    const dCol = particles.column("distance");
    const seedHiCol = particles.column("seed_hi");
    const seedLoCol = particles.column("seed_lo");
    const trajTable = res.trajectories;
    const newParticles: Particle[] = [];
    for (let i = 0; i < particles.length; i++) {
      // Trajectory columns are named `traj_{i}` for i in 0..batchSize.
      const tcol = trajTable.column(`traj_${i}`);
      newParticles.push({
        r0: r0Col[i],
        initialInfections: iiCol[i],
        weight: wCol[i],
        distance: dCol[i],
        trajectory: Array.from(tcol),
        seed: `${seedHiCol[i]}-${seedLoCol[i]}`,
      });
    }
    const nAttempts = res.batch_meta.column("n_attempts")[0];

    // Update reactive state — replace the inner array reference so Vue
    // picks it up under shallowReactive-style ownership in callers.
    const updated = [...particlesByStage.value];
    while (updated.length <= stage) updated.push([]);
    updated[stage] = [...updated[stage], ...newParticles];
    particlesByStage.value = updated;

    // Track acceptance per stage.
    const acc = [...acceptance.value];
    while (acc.length <= stage) acc.push({ nAttempts: 0, nAccepted: 0 });
    acc[stage] = {
      nAttempts: acc[stage].nAttempts + nAttempts,
      nAccepted: acc[stage].nAccepted + newParticles.length,
    };
    acceptance.value = acc;

    await saveBatch(id, stage, haveSoFar, newParticles);
    await setStatus(id, { acceptance: acc });
  }

  return {
    status,
    runId,
    runName,
    config,
    currentStage,
    particlesByStage,
    acceptance,
    errorMessage,
    select,
    run,
    pause,
    reset,
  };
}
