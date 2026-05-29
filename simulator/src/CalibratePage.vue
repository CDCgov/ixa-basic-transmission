<script setup lang="ts">
import { shallowReactive, ref, computed, onMounted, nextTick } from "vue";
import { useRouter, useRoute } from "vue-router";
import { runWasm } from "cfasim-ui/wasm";
import type { ModelOutput } from "cfasim-ui/shared";
import {
  NumberInput,
  Button,
  TextInput,
  SelectBox,
} from "cfasim-ui/components";
import type { SelectOption } from "cfasim-ui/components";
// Native <dialog> element is used for the delete-confirmation modal —
// no extra component dependency.
import { BarChart, LineChart } from "cfasim-ui/charts";
import RateEditor from "./components/RateEditor.vue";
import PriorEditor from "./components/PriorEditor.vue";
import SettingsEditor from "./components/SettingsEditor.vue";
import {
  defaultConfig,
  weightedHistogram,
  weightedKde,
  totalWeight,
  parseTargetCsv,
  acceptanceRatio,
  cumulativeToIncidence,
  incidenceToCumulative,
  densify,
  type CalibrationConfig,
} from "./composables/calibration";
import { useCalibration } from "./composables/useCalibration";
import {
  listRuns,
  createRun,
  updateConfig,
  deleteRun,
  setStatus,
  type StoredRun,
} from "./composables/calibrationStorage";

const WASM_NAME = "ixa_basic_transmission";

// Flat top-level reactive state, same rule as Page.vue: only ever
// replace whole top-level keys; never mutate nested fields in place
// under shallowReactive (DataCloneError + missed reactivity).
const seed = defaultConfig();
const defaults = {
  infectionRate: seed.modelContext.infectionRate,
  population: seed.modelContext.population,
  maxTime: seed.modelContext.maxTime,
  settings: seed.modelContext.settings,
  initialInfections: seed.initialInfections,
  priors: seed.priors,
  stagesText: seed.stages.join(","),
  nParticles: seed.nParticles,
  batchSize: seed.batchSize,
  seed: seed.seed,
  varianceFactor: seed.varianceFactor ?? 2.0,
  probKeepSeed: seed.probKeepSeed ?? 0.0,
  distanceMetric: (seed.distanceMetric ?? "cumulative") as
    | "cumulative"
    | "daily",
  // Manual entry by default — the user can populate the table from a
  // model run or a CSV via the buttons next to the editor. `targetMode`
  // is provenance only (what last filled the table); it no longer drives
  // any behaviour on Start.
  targetMode: "manual" as TargetMode,
  csvFilename: "",
  runName: defaultRunName(),
};

function defaultRunName(): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getMonth() + 1}/${d.getDate()} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
type Params = typeof defaults;
const params = shallowReactive(structuredClone(defaults));
function setParam<K extends keyof Params>(key: K, value: Params[K]): void {
  params[key] = value;
}

// Observed data isn't part of `params` because it shouldn't round-trip
// through the URL (potentially long; synthetic mode regenerates; CSV
// upload is session-local).
// The editor's source of truth: one (day, value) row per observation.
// Days are user-editable and may have gaps (e.g. weekly surveillance);
// `observed` below densifies them into the per-day array the calibrator
// consumes, zero-filling any missing days within [1, maxTime].
const targetRows = ref<{ day: number; value: number }[]>([]);

function setRowsFromDense(arr: number[]) {
  targetRows.value = arr.map((value, i) => ({ day: i + 1, value }));
}

// `densify` lives in composables/calibration.ts so it's unit-tested
// directly.

// Whether the target data is expressed (and compared) as cumulative case
// totals vs daily incidence — the single shared toggle that also drives
// the distance scale (`distanceMetric`).
const isCumulative = computed(() => params.distanceMetric === "cumulative");

// Per-day dense series fed to the calibrator. Always floor(maxTime) long
// so it lines up with each particle's simulated trajectory. The typed
// value (daily or cumulative, per scale) lands at its observed-day index;
// gap days are 0 but never scored — the gap-aware L1 only reads
// `observedDays`, and Rust cumulates the simulated side to match.
const observed = computed<number[]>(() =>
  densify(targetRows.value, Math.floor(params.maxTime)),
);

// 1-based days the target actually contains (unique, in-range, sorted) —
// the gap-aware L1 distance is scored only at these days (on whichever
// scale is active), so gaps are skipped rather than scored as zero. Kept
// consistent with `densify` (same [1, floor(maxTime)] clamp; dups collapse).
const observedDays = computed<number[]>(() => {
  const maxDay = Math.floor(params.maxTime);
  const seen = new Set<number>();
  for (const r of targetRows.value) {
    if (r.day >= 1 && r.day <= maxDay) seen.add(r.day);
  }
  return [...seen].sort((a, b) => a - b);
});

// Provenance of the data currently in the table. Editing a value flips
// it back to "manual" (see markEdited).
type TargetMode = "synthetic" | "csv" | "manual";

const metricOptions: SelectOption[] = [
  { value: "cumulative", label: "Cumulative incidence" },
  { value: "daily", label: "Daily incidence" },
];

// Axis / column label tracking the active scale, so the table, the target
// plot, and the per-stage trajectory overlay all read consistently.
const valueLabel = computed(() =>
  isCumulative.value ? "Cumulative cases" : "Incident cases",
);

// Human label for the selected scale (the matching option label), used in
// the read-only readout shown once a run locks the choice.
const metricLabel = computed(
  () =>
    metricOptions.find((o) => o.value === params.distanceMetric)?.label ??
    params.distanceMetric,
);

function parseStages(text: string): number[] {
  return text
    .split(",")
    .map((s) => Number(s.trim()))
    .filter((n) => Number.isFinite(n) && n > 0 && n < 1);
}

const config = computed<CalibrationConfig>(() => ({
  modelContext: {
    infectionRate: params.infectionRate,
    population: params.population,
    maxTime: params.maxTime,
    settings: params.settings,
  },
  initialInfections: params.initialInfections,
  priors: params.priors,
  stages: parseStages(params.stagesText),
  nParticles: params.nParticles,
  batchSize: params.batchSize,
  seed: params.seed,
  varianceFactor: params.varianceFactor,
  probKeepSeed: params.probKeepSeed,
  observed: observed.value,
  observedDays: observedDays.value,
  distanceMetric: params.distanceMetric,
  target:
    params.targetMode === "synthetic"
      ? { mode: "synthetic" }
      : params.targetMode === "csv"
        ? { mode: "csv", filename: params.csvFilename }
        : { mode: "manual" },
}));

const runner = useCalibration();

// Params are editable only for a not-yet-started (idle) run. Once a run
// is running/paused/complete/error its config is frozen — the user must
// create a new run to change anything.
const paramsLocked = computed(() => runner.status.value !== "idle");

// "Create new run" mints a fresh idle run; "Start" then runs the selected
// idle run (the two are separate actions). Start only makes sense once a
// run exists and is still idle (not running/paused/complete/error).
const canStart = computed(
  () => runner.runId.value !== null && runner.status.value === "idle",
);
// Make "Create new run" the primary CTA when there's no other primary
// action on screen (no run yet, or the selected run finished); demote it
// to secondary while Pause/Resume are the focus. (It's hidden entirely
// when a selected run is idle — Start is the action there.)
const createVariant = computed(() =>
  runner.status.value === "running" || runner.status.value === "paused"
    ? "secondary"
    : "primary",
);

// --- Saved-run management ----------------------------------------------------

const savedRuns = ref<StoredRun[]>([]);
async function refreshRuns() {
  savedRuns.value = await listRuns();
}

// --- Inline run-name edit (double-click on a list item) --------------------

const editingRunId = ref<string | null>(null);
const editingName = ref<string>("");
const editingInvalid = ref<boolean>(false);

function startEditing(run: StoredRun) {
  editingRunId.value = run.id;
  editingName.value = run.name;
  editingInvalid.value = false;
}

function cancelEdit() {
  editingRunId.value = null;
  editingName.value = "";
  editingInvalid.value = false;
}

async function commitEdit() {
  const id = editingRunId.value;
  if (!id) return;
  const trimmed = editingName.value.trim();
  if (trimmed.length === 0) {
    // Refuse to save; keep the editor open with an invalid hint.
    editingInvalid.value = true;
    return;
  }
  // Sync the runner's own copy so a watching component (status line,
  // etc) stays consistent without a reload.
  if (runner.runId.value === id) {
    runner.runName.value = trimmed;
    params.runName = trimmed;
  }
  await setStatus(id, { name: trimmed });
  editingRunId.value = null;
  editingName.value = "";
  editingInvalid.value = false;
  await refreshRuns();
}

const router = useRouter();
const route = useRoute();

/// Reflect the active runId in the URL so a refresh / shared link picks
/// the same run back up. We use `router.replace` (not `push`) so the
/// browser back button doesn't fill with intermediate run states.
function syncRunIdToUrl(id: string | null) {
  const next = { ...route.query };
  if (id) next.runId = id;
  else delete next.runId;
  router.replace({ query: next });
}

onMounted(async () => {
  await refreshRuns();
  const initial = String(route.query.runId ?? "");
  if (initial && savedRuns.value.some((r) => r.id === initial)) {
    selectedRunId.value = initial;
    await onSelectRun(initial);
  }
});

function shortStatus(r: StoredRun): string {
  if (r.status === "complete") return "done";
  if (r.status === "idle") return "idle";
  // Running / paused / error all benefit from the stage hint.
  return `${r.status} ${r.currentStage}`;
}

/// Like `shortStatus` but reads live runner state when the row IS the
/// currently active run — so the badge updates per batch instead of
/// staying frozen at the last persisted snapshot. savedRuns only
/// re-reads from IDB on explicit actions (create / select / delete) so
/// without this override the list status would lag behind reality.
function liveStatus(r: StoredRun): string {
  if (r.id === runner.runId.value) {
    const s = runner.status.value;
    if (s === "complete") return "done";
    if (s === "idle") return "idle";
    return `${s} ${runner.currentStage.value}`;
  }
  return shortStatus(r);
}

const selectedRunId = ref<string | null>(null);

async function createNewRun() {
  cancelEdit();
  // Mint a fresh default name so two "+ New" clicks land distinct
  // timestamps (otherwise the page-load default would be reused).
  params.runName = defaultRunName();
  const id = await createRun(config.value, params.runName);
  selectedRunId.value = id;
  syncRunIdToUrl(id);
  await runner.select(id);
  await refreshRuns();
}

async function onSelectRun(id: string | number) {
  const v = String(id);
  selectedRunId.value = v;
  syncRunIdToUrl(v);
  await runner.select(v);
  // Snap sidebar config onto the resumed run's stored config so the
  // user can read what was running. Keep `observed` since each run
  // carries its own snapshot.
  const c = runner.config.value;
  if (c) {
    setParam("infectionRate", c.modelContext.infectionRate);
    setParam("population", c.modelContext.population);
    setParam("maxTime", c.modelContext.maxTime);
    setParam("settings", c.modelContext.settings);
    // Default 10 for runs persisted before initialInfections moved into
    // the top-level config (was a target.synthetic field).
    setParam("initialInfections", c.initialInfections ?? 10);
    setParam("priors", c.priors);
    setParam("stagesText", c.stages.join(","));
    setParam("nParticles", c.nParticles);
    setParam("batchSize", c.batchSize);
    setParam("seed", c.seed);
    setParam("varianceFactor", c.varianceFactor ?? 2.0);
    setParam("probKeepSeed", c.probKeepSeed ?? 0.0);
    setParam("distanceMetric", c.distanceMetric ?? "cumulative");
    // Rebuild the editable rows. Prefer `observedDays` so gaps survive a
    // reload; fall back to a contiguous dense expansion for older rows.
    if (c.observedDays && c.observedDays.length) {
      targetRows.value = c.observedDays.map((d) => ({
        day: d,
        value: c.observed[d - 1] ?? 0,
      }));
    } else {
      setRowsFromDense(c.observed);
    }
    if (c.target.mode === "synthetic") {
      setParam("targetMode", "synthetic");
    } else if (c.target.mode === "csv") {
      setParam("targetMode", "csv");
      setParam("csvFilename", c.target.filename);
    } else {
      setParam("targetMode", "manual");
    }
    setParam("runName", runner.runName.value);
  }
}

// --- Target data generation / upload ----------------------------------------

async function generateSyntheticObserved(): Promise<number[]> {
  // Reuse the existing `simulate` wasm export (a single deterministic
  // sim with truth params) and convert the cumulative trajectory into
  // daily incidence via the same helper the ABC distance metric uses.
  const args = JSON.stringify({
    infectionRate: params.infectionRate,
    population: params.population,
    initialInfections: params.initialInfections,
    seed: 0,
    maxTime: params.maxTime,
    nSimulations: 1,
    settings: params.settings,
  });
  const res = (await runWasm(WASM_NAME, "simulate", args)) as Record<
    string,
    ModelOutput
  >;
  return cumulativeToIncidence(res.series.column("cumulative_infections_0"));
}

/// Fill the table by running the model once with the sidebar's
/// parameters (a one-shot populate, not a live link — the user can edit
/// the result afterward).
const generating = ref(false);
async function generateFromParameters() {
  generating.value = true;
  try {
    // generateSyntheticObserved returns daily incidence; lift it onto the
    // cumulative scale when that's how the table is expressed.
    const daily = await generateSyntheticObserved();
    setRowsFromDense(isCumulative.value ? incidenceToCumulative(daily) : daily);
    setParam("targetMode", "synthetic");
  } catch (e) {
    alert(`Generate failed: ${e instanceof Error ? e.message : String(e)}`);
  } finally {
    generating.value = false;
  }
}

const csvInput = ref<HTMLInputElement | null>(null);
function triggerCsvUpload() {
  csvInput.value?.click();
}

/// Empty the table back to a blank manual slate.
function clearData() {
  targetRows.value = [];
  setParam("targetMode", "manual");
  setParam("csvFilename", "");
}

async function onUploadCsv(ev: Event) {
  const input = ev.target as HTMLInputElement;
  const f = input.files?.[0];
  if (!f) return;
  const text = await f.text();
  try {
    setRowsFromDense(parseTargetCsv(text));
    setParam("targetMode", "csv");
    setParam("csvFilename", f.name);
  } catch (e) {
    alert(`CSV parse failed: ${e instanceof Error ? e.message : String(e)}`);
  } finally {
    // Allow re-uploading the same file (change won't fire otherwise).
    input.value = "";
  }
}

/// Hand-editing the table makes the data user-curated, so record it as
/// `manual` provenance regardless of how it was first populated.
function markEdited() {
  if (params.targetMode !== "manual") setParam("targetMode", "manual");
}

// --- Editable observed-data table -------------------------------------------
// `targetRows` is reassigned wholesale (never mutated in place) so the
// `ref` reactivity fires and `observed` + the plot + per-stage overlays
// recompute.

function setRowDay(i: number, day: number) {
  const next = targetRows.value.slice();
  next[i] = {
    ...next[i],
    day: Number.isFinite(day) ? Math.max(1, Math.round(day)) : next[i].day,
  };
  targetRows.value = next;
  markEdited();
}

function setRowValue(i: number, value: number) {
  const next = targetRows.value.slice();
  next[i] = {
    ...next[i],
    value: Number.isFinite(value) ? Math.max(0, Math.round(value)) : 0,
  };
  targetRows.value = next;
  markEdited();
}

const sheetScroll = ref<HTMLElement | null>(null);

async function addObservedRow() {
  // Default the new row's day to one past the current max so appended
  // rows stay contiguous unless the user edits them.
  const maxDay = targetRows.value.reduce((m, r) => Math.max(m, r.day), 0);
  targetRows.value = [...targetRows.value, { day: maxDay + 1, value: 0 }];
  markEdited();
  // The new row has the largest day, so it lands at the bottom after the
  // sorted render — scroll the sheet down to reveal it.
  await nextTick();
  const el = sheetScroll.value;
  if (el) el.scrollTop = el.scrollHeight;
}

function removeObservedRow(i: number) {
  targetRows.value = targetRows.value.filter((_, j) => j !== i);
  markEdited();
}

/// Keep the table ordered by day. Called on a day field's commit (blur),
/// not on every keystroke, so a row being edited doesn't jump around
/// mid-typing — the plot already renders sorted regardless.
function sortRowsByDay() {
  targetRows.value = [...targetRows.value].sort((a, b) => a.day - b.day);
}

// --- Run lifecycle -----------------------------------------------------------

const errorMessage = ref<string | undefined>();

async function startRun() {
  errorMessage.value = undefined;
  // JS-side validation up front so the user sees a readable error
  // instead of a wasm trap from `model::run` deeper in the loop.
  const p = params.priors;
  if (p.r0Hi <= p.r0Lo) {
    errorMessage.value = "R₀ upper must be greater than lower.";
    return;
  }
  if (p.initialInfectionsHi < p.initialInfectionsLo) {
    errorMessage.value = "Initial-infections upper must be ≥ lower.";
    return;
  }
  if (p.initialInfectionsHi > params.population) {
    errorMessage.value = `Initial-infections upper (${p.initialInfectionsHi}) must be ≤ population (${params.population}).`;
    return;
  }
  if (params.batchSize < 1) {
    errorMessage.value = "Batch size must be at least 1.";
    return;
  }
  if (parseStages(params.stagesText).length === 0) {
    errorMessage.value =
      "Error schedule must list at least one comma-separated value in (0, 1).";
    return;
  }
  try {
    if (targetRows.value.length === 0) {
      errorMessage.value =
        "Add target data before starting — enter rows by hand, generate from parameters, or upload a CSV.";
      return;
    }
    // Days are clamped to [1, floor(maxTime)] when densified; flag any
    // out-of-range rows so the user fixes them rather than silently
    // losing those observations.
    const maxDay = Math.floor(params.maxTime);
    const offending = targetRows.value.find(
      (r) => r.day < 1 || r.day > maxDay,
    );
    if (offending) {
      errorMessage.value = `Day ${offending.day} is outside 1…${maxDay} (max time). Adjust the day or increase max time.`;
      return;
    }
    // Cumulative case totals are monotone by definition — a decrease is a
    // data-entry mistake (and would calibrate against an impossible
    // target). Flag it so the user fixes the entry instead.
    if (isCumulative.value) {
      const sorted = [...targetRows.value].sort((a, b) => a.day - b.day);
      for (let i = 1; i < sorted.length; i++) {
        if (sorted[i].value < sorted[i - 1].value) {
          errorMessage.value = `Cumulative cases must be non-decreasing, but day ${sorted[i].day} (${sorted[i].value}) is less than day ${sorted[i - 1].day} (${sorted[i - 1].value}).`;
          return;
        }
      }
    }
    // Mint a fresh run when there's nothing selected OR when the
    // selected run is already finished. Overwriting a complete/error
    // run's config would silently corrupt its history and the loop
    // would no-op (useCalibration.run returns early on those statuses).
    // `idle` (the "+ New" placeholder) and `paused` (resumable) reuse
    // the existing run.
    const status = runner.status.value;
    const needsFresh =
      !runner.runId.value || status === "complete" || status === "error";
    if (needsFresh) {
      params.runName = defaultRunName();
      const id = await createRun(config.value, params.runName);
      selectedRunId.value = id;
      syncRunIdToUrl(id);
      await runner.select(id);
    }
    // Push the sidebar config (priors, target data, etc.) onto the
    // selected run's IDB row so the loop picks up the user's latest
    // edits — they may have tweaked priors after clicking "+ New".
    await updateConfig(runner.runId.value!, config.value);
    await refreshRuns();
    await runner.run();
  } catch (e) {
    errorMessage.value = e instanceof Error ? e.message : String(e);
  }
}

function pauseRun() {
  runner.pause();
}

async function resumeRun() {
  await runner.run();
}

// --- Delete-with-confirmation ----------------------------------------------

const pendingDelete = ref<StoredRun | null>(null);
const deleteDialog = ref<HTMLDialogElement | null>(null);

function requestDelete(run: StoredRun) {
  pendingDelete.value = run;
  deleteDialog.value?.showModal();
}

function cancelDelete() {
  pendingDelete.value = null;
  deleteDialog.value?.close();
}

async function confirmDelete() {
  const target = pendingDelete.value;
  pendingDelete.value = null;
  deleteDialog.value?.close();
  if (!target) return;
  // If we're deleting the currently-active run, clear runner state +
  // URL first so the page doesn't keep referencing a gone IDB row.
  if (runner.runId.value === target.id) {
    runner.reset();
    selectedRunId.value = null;
    syncRunIdToUrl(null);
  }
  await deleteRun(target.id);
  await refreshRuns();
}

// --- Diagnostics: per-stage parameter trace grid ----------------------------

interface TrajectorySeries {
  x: number[];
  data: number[];
  color: string;
  strokeWidth: number;
  dots: boolean;
}

/// KDE in BarChart `summaryLines` shape: `x` is in category-index
/// space (0 = first bar center, N-1 = last), so it's drawn on top of
/// the corresponding histogram. The line autoscales independently of
/// the bar y-axis — no bin-width rescaling needed.
interface KdeOverlay {
  x: number[];
  data: number[];
}

/// Two-series overlay payload for a BarChart with `layout="overlay"`:
/// the posterior bins drawn on top of the (flat) prior bins. Categories
/// are shared.
interface OverlayBarSeries {
  categories: string[];
  prior: number[];
  posterior: number[];
}

interface StageTrace {
  stage: number;
  stageLabel: string;
  n: number;
  acceptance: number | null;
  r0Bins: OverlayBarSeries;
  iiBins: OverlayBarSeries;
  r0KdeOverlay: KdeOverlay;
  trajectorySeries: TrajectorySeries[];
}

function stageLabelFor(s: number): string {
  return s === 0 ? "Prior (∞)" : `Stage ${s}`;
}

/// Per-stage colors for the all-stages overlay: the prior (always the
/// first rendered stage) is grey to set it apart as the un-calibrated
/// baseline; the perturbation stages run a light→dark blue ramp so the
/// contraction toward the final posterior reads as deepening blue. `i` is
/// the index into the present stages, `n` their count.
function stageColor(i: number, n: number): string {
  if (i === 0) return "#9ca3af"; // prior → grey
  const t = n > 1 ? i / (n - 1) : 0;
  // Light→dark blue: floor at ~18% lightness so the final stage reads as a
  // deep navy, clearly distinct from the penultimate one.
  return `hsl(222, 75%, ${Math.round(70 - t * 52)}%)`;
}

const stageTraces = computed<StageTrace[]>(() => {
  const stages = runner.particlesByStage.value;
  const acc = runner.acceptance.value;
  const out: StageTrace[] = [];
  // Bin width — used to project KDE x positions (in value space) into
  // BarChart category-index space so the overlay sits over the right
  // bars. category_idx = (x - lo) / binWidth - 0.5.
  const r0BinWidth = (params.priors.r0Hi - params.priors.r0Lo) / 18;
  for (let s = 0; s < stages.length; s++) {
    const ps = stages[s];
    if (!ps || ps.length === 0) continue;
    const total = totalWeight(ps);
    const normalizedWeights = ps.map((p) =>
      total > 0 ? p.weight / total : 1 / ps.length,
    );
    const r0Bins = weightedHistogram(
      ps.map((p) => p.r0),
      normalizedWeights,
      18,
      params.priors.r0Lo,
      params.priors.r0Hi,
    );
    const iiRange = Math.max(
      1,
      params.priors.initialInfectionsHi - params.priors.initialInfectionsLo + 1,
    );
    const iiBins = weightedHistogram(
      ps.map((p) => p.initialInfections),
      normalizedWeights,
      iiRange,
      params.priors.initialInfectionsLo - 0.5,
      params.priors.initialInfectionsHi + 0.5,
    );
    // Weighted Gaussian KDE for r0 — shown beside the bars as a smoothed,
    // bin-independent rendering of the same posterior. No KDE for
    // `initial_infections`: it's discrete and the perturbation kernel
    // never moves it, so a smooth curve would imply density between
    // integers and exploration that the algorithm structurally doesn't
    // perform.
    const r0Kde = weightedKde(
      ps.map((p) => p.r0),
      normalizedWeights,
      params.priors.r0Lo,
      params.priors.r0Hi,
      120,
    );
    const a = acc[s];
    // Particle trajectories as faint gray lines; observed (if loaded) as
    // a bolder teal overlay so the eye can compare.
    // Particle trajectories as faint gray lines. Stored as daily
    // incidence; lift onto the cumulative scale for display when that's
    // the active representation so they line up with the observed overlay.
    const trajectorySeries: TrajectorySeries[] = ps
      .filter((p) => p.trajectory.length > 0)
      .map((p) => {
        const data = isCumulative.value
          ? incidenceToCumulative(p.trajectory)
          : p.trajectory;
        return {
          x: data.map((_, i) => i + 1),
          data,
          color: "rgba(100, 116, 139, 0.35)",
          strokeWidth: 1,
          dots: false,
        };
      });
    // Observed target as a bolder teal line + dots through the days it
    // actually contains (sparse — connects real observations by day, no
    // densified fill). Values are already on the active scale (the typed
    // daily or cumulative counts).
    if (sortedRows.value.length) {
      trajectorySeries.push({
        x: sortedRows.value.map((r) => r.day),
        data: sortedRows.value.map((r) => r.value),
        color: "#14b8a6",
        strokeWidth: 2.5,
        dots: true,
      });
    }
    out.push({
      stage: s,
      stageLabel: stageLabelFor(s),
      n: ps.length,
      acceptance: a ? acceptanceRatio(a.nAccepted, a.nAttempts) : null,
      // Bar heights are bin weights summing to 1 across the panel
      // ("probability mass per bin").
      // Prior series: uniform PDF integrates to 1 over its support, so
      // each of N bins gets mass 1/N. Drawn under the posterior so the
      // "departure from flat" pops visually.
      r0Bins: {
        categories: r0Bins.map((b) => b.center.toFixed(2)),
        prior: r0Bins.map(() => 1 / r0Bins.length),
        posterior: r0Bins.map((b) => b.weight),
      },
      iiBins: {
        categories: iiBins.map((b) => b.center.toFixed(0)),
        prior: iiBins.map(() => 1 / iiBins.length),
        posterior: iiBins.map((b) => b.weight),
      },
      // BarChart summaryLines autoscale to their own value extent, so
      // the y values stay raw (density). x is projected into category-
      // index space.
      r0KdeOverlay: {
        x: r0Kde.x.map((v) => (v - params.priors.r0Lo) / r0BinWidth - 0.5),
        data: r0Kde.y,
      },
      trajectorySeries,
    });
  }
  return out;
});

/// All-stages overlay for each calibrated parameter: one KDE line per
/// stage for the continuous `r0`, one posterior-histogram series per stage
/// (overlaid bars) for the discrete `initial_infections`. Stages are
/// colored prior-light → posterior-dark so the contraction is legible on a
/// single axis. Re-derived from the raw particles (the per-stage
/// `r0KdeOverlay` is projected into category-index space, so it can't be
/// reused for a value-axis line here).
interface OverlaySeries {
  x: number[];
  data: number[];
  color: string;
  strokeWidth: number;
  dots: boolean;
  legend: string;
}
interface OverlayArea {
  upper: number[];
  lower: number[];
  x: number[];
  color: string;
  opacity: number;
}
interface BarOverlaySeries {
  data: number[];
  color: string;
  legend: string;
  opacity: number;
}
const stageOverlays = computed<{
  r0Series: OverlaySeries[];
  r0Areas: OverlayArea[];
  iiCategories: string[];
  iiSeries: BarOverlaySeries[];
}>(() => {
  const stages = runner.particlesByStage.value;
  const present: number[] = [];
  for (let s = 0; s < stages.length; s++) {
    if (stages[s] && stages[s].length) present.push(s);
  }
  const n = present.length;
  const { r0Lo, r0Hi, initialInfectionsLo: iiLo, initialInfectionsHi: iiHi } =
    params.priors;
  const iiRange = Math.max(1, iiHi - iiLo + 1);

  const r0Series: OverlaySeries[] = [];
  const r0Areas: OverlayArea[] = [];
  const iiSeries: BarOverlaySeries[] = [];
  let iiCategories: string[] = [];
  present.forEach((s, idx) => {
    const ps = stages[s];
    const total = totalWeight(ps);
    const w = ps.map((p) => (total > 0 ? p.weight / total : 1 / ps.length));
    const legend = stageLabelFor(s);
    const color = stageColor(idx, n);
    // Ramp opacity up by stage so later stages — especially the final
    // posterior — render progressively more opaque (darker) and stand out
    // over the lighter early stages.
    const t = n > 1 ? idx / (n - 1) : 1;
    const kde = weightedKde(
      ps.map((p) => p.r0),
      w,
      r0Lo,
      r0Hi,
      120,
    );
    // Filled area under each KDE (lower = 0) plus a thin line on top for a
    // crisp edge and the legend entry (Area carries no legend).
    r0Areas.push({
      upper: kde.y,
      lower: kde.y.map(() => 0),
      x: kde.x,
      color,
      opacity: 0.2 + t * 0.45,
    });
    r0Series.push({
      x: kde.x,
      data: kde.y,
      color,
      strokeWidth: 2,
      dots: false,
      legend,
    });
    const bins = weightedHistogram(
      ps.map((p) => p.initialInfections),
      w,
      iiRange,
      iiLo - 0.5,
      iiHi + 0.5,
    );
    if (idx === 0) iiCategories = bins.map((b) => b.center.toFixed(0));
    // Overlaid bars with translucent fills (not `multiply`) so overlapping
    // stages blend gently instead of compounding toward black; later
    // stages get more opacity so the final posterior reads darkest.
    iiSeries.push({
      data: bins.map((b) => b.weight),
      color,
      legend,
      opacity: 0.45 + t * 0.45,
    });
  });
  return { r0Series, r0Areas, iiCategories, iiSeries };
});

const statusLabel = computed(() => {
  const s = runner.status.value;
  if (s === "idle") return "Idle";
  if (s === "complete") return "Complete";
  if (s === "error") return `Error: ${runner.errorMessage.value ?? "unknown"}`;
  const stage = runner.currentStage.value;
  const c = runner.config.value;
  const have = runner.particlesByStage.value[stage]?.length ?? 0;
  const target = c?.nParticles ?? params.nParticles;
  const totalStages = c ? c.stages.length + 1 : parseStages(params.stagesText).length + 1;
  const stageLabel = stage === 0 ? "0 (prior)" : String(stage);
  return `${s === "paused" ? "Paused" : "Running"} — stage ${stageLabel}/${totalStages - 1} — ${have}/${target} particles`;
});

const summaryRows = computed(() =>
  stageTraces.value.map((t) => ({
    stage: t.stageLabel,
    particles: t.n,
    // Stage 0's acceptance is trivially 1.0 (threshold = ∞), so the
    // ratio is vacuous — render as "—" instead of "100%" to avoid
    // implying it's a meaningful diagnostic.
    acceptance:
      t.stage === 0 || t.acceptance === null
        ? "—"
        : `${(t.acceptance * 100).toFixed(1)}%`,
  })),
);

// Rows sorted by day — shared by the plot series and the gap shading so
// their point indices line up (areaSections index into series[0]).
const sortedRows = computed(() =>
  [...targetRows.value].sort((a, b) => a.day - b.day),
);

// Plot the rows directly (sorted by day) as a line + dots so gaps read as
// spaced points rather than zero-filled valleys (the gap shading below
// marks the ranges the line bridges).
const observedSeries = computed(() => {
  if (sortedRows.value.length === 0) return [];
  return [
    {
      x: sortedRows.value.map((r) => r.day),
      data: sortedRows.value.map((r) => r.value),
      color: "#0f766e",
      strokeWidth: 2,
      dots: true,
    },
  ];
});

// Light-red bands over the day ranges with no observation — the same
// gaps the distance skips. `areaSections` index into series[0]'s points
// (mapped through its `x`/day values), so a section spanning indices
// i→i+1 shades from one observed day to the next whenever they're more
// than a day apart.
const observedGapSections = computed(() => {
  const rows = sortedRows.value;
  const sections: {
    startIndex: number;
    endIndex: number;
    color: string;
    opacity: number;
    strokeWidth: number;
    legend: false;
  }[] = [];
  for (let i = 0; i < rows.length - 1; i++) {
    if (rows[i + 1].day - rows[i].day > 1) {
      sections.push({
        startIndex: i,
        endIndex: i + 1,
        color: "#dc2626",
        opacity: 0.1,
        // 0 = no vertical boundary lines; just the translucent fill.
        strokeWidth: 0,
        legend: false,
      });
    }
  }
  return sections;
});
</script>

<template>
  <Teleport to="#model-sidebar">
    <div class="sidebar-header">
      <div class="run-list">
        <div class="run-list__header">
          <span>Runs</span>
          <Button variant="secondary" @click="createNewRun">+ New</Button>
        </div>
        <p v-if="!savedRuns.length" class="run-list__empty">
          No saved runs yet. Configure below and click Start.
        </p>
        <ul v-else class="run-list__items">
          <li v-for="r in savedRuns" :key="r.id" class="run-list__item"
            :class="{ 'run-list__item--active': r.id === selectedRunId }">
            <form v-if="editingRunId === r.id" class="run-list__edit-form" @submit.prevent="commitEdit">
              <input v-model="editingName" class="run-list__edit-input"
                :class="{ 'run-list__edit-input--invalid': editingInvalid }" :aria-label="`Rename ${r.name}`"
                :title="editingInvalid ? 'Name cannot be empty' : undefined" autofocus
                @keydown.escape.prevent="cancelEdit" @input="editingInvalid = false" @blur="commitEdit" />
            </form>
            <button v-else type="button" class="run-list__select"
              :title="`${r.name} · ${liveStatus(r)} (double-click to rename)`" @click="onSelectRun(r.id)"
              @dblclick="startEditing(r)">
              <span class="run-list__name">{{ r.name }}</span>
              <span class="run-list__status">{{ liveStatus(r) }}</span>
            </button>
            <button v-if="editingRunId !== r.id" type="button" class="run-list__delete" :title="`Delete ${r.name}`"
              :aria-label="`Delete ${r.name}`" @click="requestDelete(r)">
              ×
            </button>
          </li>
        </ul>
      </div>
    </div>

    <p v-if="paramsLocked" class="cal-locked-banner">
      This run is read-only. Create a new run to change parameters.
    </p>

    <fieldset class="cal-fieldset" :disabled="paramsLocked">
      <section class="cal-section">
        <h3>Model</h3>
        <RateEditor v-model="params.infectionRate" />
        <NumberInput v-model="params.population" label="Population" :min="100" />
        <NumberInput v-model="params.initialInfections" label="Initial infections (truth, used in synthetic mode)"
          :min="1" />
        <NumberInput v-model="params.maxTime" label="Max time (days)" :min="1" />
        <SettingsEditor v-model="params.settings" />
      </section>

      <section class="cal-section">
        <h3>Priors</h3>
        <PriorEditor v-model="params.priors" />
      </section>

      <section class="cal-section">
        <h3>Calibration controls</h3>
        <NumberInput v-model="params.nParticles" label="Particles per stage" :min="10" />
        <NumberInput v-model="params.batchSize" label="Batch size" :min="1" />
        <NumberInput v-model="params.seed" label="Seed" :min="0" />
        <NumberInput v-model="params.varianceFactor"
          label="Kernel variance factor (kernel SD = √(factor × Var); 2.0 recommended)" :min="0.01" :step="0.1" />
        <NumberInput v-model="params.probKeepSeed" label="Seed-keep probability (0 = always fresh)" :min="0" :max="1"
          :step="0.01" />
        <TextInput v-model="params.stagesText" label="Error schedule (comma-separated, 0 < r < 1)" />
      </section>
    </fieldset>

    <div class="sidebar-controls">
      <Button v-if="canStart" @click="startRun">Start</Button>
      <Button v-else-if="runner.status.value === 'running'" variant="secondary" @click="pauseRun">
        Pause
      </Button>
      <Button v-else-if="runner.status.value === 'paused'" variant="secondary" @click="resumeRun">
        Resume
      </Button>
      <!-- Hidden while a selected run is idle (Start is the action there);
           shown with no run yet, or once a run is paused/complete/error. -->
      <Button
        v-if="!canStart"
        :variant="createVariant"
        :disabled="runner.status.value === 'running'"
        @click="createNewRun"
      >
        Create new run
      </Button>
    </div>
  </Teleport>

  <dialog ref="deleteDialog" class="confirm-dialog" @cancel="cancelDelete">
    <h3 class="confirm-dialog__title">Delete run?</h3>
    <p class="confirm-dialog__body">
      <template v-if="pendingDelete">
        “{{ pendingDelete.name }}” — {{ liveStatus(pendingDelete) }}.
        This can't be undone.
      </template>
    </p>
    <div class="confirm-dialog__actions">
      <Button variant="secondary" @click="cancelDelete">Cancel</Button>
      <Button @click="confirmDelete">Delete</Button>
    </div>
  </dialog>

  <h1>Calibration</h1>
  <p>
    Calibrates the transmission model to an observed incidence series via
    ABC-SMC. Pick priors, set an error schedule, then watch the posterior
    tighten across stages. Runs persist in your browser, so even if you
    close the tab, you can resume later.
  </p>
  <p v-if="errorMessage" class="error">{{ errorMessage }}</p>
  <p class="status">{{ statusLabel }}</p>

  <section class="cal-metric">
    <h2>Distance metric</h2>
    <p class="cal-metric__desc">
      Fit is scored with a <strong>gap-aware L1 distance</strong> — the sum
      of absolute differences between the target and the model output,
      taken only on the days the target actually contains (gaps are
      skipped, not counted as zero). Choose whether that comparison happens
      on the cumulative or daily scale; the target table and the model
      output are both expressed that way.
    </p>
    <!-- The scale is part of the run's frozen config: switching it would
         reinterpret a started run's stored target. Once locked, show the
         chosen scale read-only instead of the (reka-ui) combobox, which a
         disabled fieldset can't actually disable. -->
    <SelectBox v-if="!paramsLocked" label="Express target data as" :options="metricOptions"
      :model-value="params.distanceMetric"
      @update:model-value="(v) => setParam('distanceMetric', String(v) as 'cumulative' | 'daily')" />
    <p v-else class="cal-metric__locked">
      Scale: <strong>{{ metricLabel }}</strong>
      <span class="cal-hint">(locked for this run)</span>
    </p>
  </section>

  <section class="cal-target">
    <h2>Target data</h2>

    <div v-if="targetRows.length" class="cal-target__plot">
      <LineChart :series="observedSeries" :area-sections="observedGapSections" :height="180" x-label="Day"
        :y-label="valueLabel" :menu="false" tooltip-trigger="hover" />
    </div>

    <div v-if="!paramsLocked" class="obs-editor">
      <div class="obs-editor__actions">
        <Button variant="secondary" :disabled="generating" @click="generateFromParameters">
          {{ generating ? "Generating…" : "Generate from parameters" }}
        </Button>
        <Button variant="secondary" @click="triggerCsvUpload">
          Upload CSV
        </Button>
        <Button variant="secondary" :disabled="!targetRows.length" @click="clearData">
          Clear
        </Button>
        <input ref="csvInput" type="file" accept=".csv,text/csv" class="obs-editor__file" @change="onUploadCsv" />
      </div>
      <fieldset class="cal-fieldset obs-editor__body">
        <div ref="sheetScroll" class="obs-sheet-scroll">
          <table class="obs-sheet">
            <thead>
              <tr>
                <th scope="col">Day</th>
                <th scope="col">{{ valueLabel }}</th>
                <th scope="col"><span class="sr-only">Remove</span></th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="(row, i) in targetRows" :key="i">
                <td>
                  <input type="number" min="1" step="1" class="obs-cell" :aria-label="`Day for row ${i + 1}`"
                    :value="row.day" @input="(e) => setRowDay(i, Number((e.target as HTMLInputElement).value))"
                    @change="sortRowsByDay" />
                </td>
                <td>
                  <input type="number" min="0" step="1" class="obs-cell" :aria-label="`${valueLabel} for row ${i + 1}`"
                    :value="row.value" @input="(e) => setRowValue(i, Number((e.target as HTMLInputElement).value))" />
                </td>
                <td class="obs-sheet__rm">
                  <button type="button" class="obs-rm" :aria-label="`Remove row ${i + 1}`"
                    @click="removeObservedRow(i)">
                    ×
                  </button>
                </td>
              </tr>
              <tr v-if="!targetRows.length">
                <td colspan="3" class="cal-hint">No data yet.</td>
              </tr>
            </tbody>
          </table>
        </div>
        <Button variant="secondary" @click="addObservedRow">Add row</Button>
      </fieldset>
    </div>
  </section>

  <section v-if="stageTraces.length" class="overlay-section">
    <h2>Posterior across stages</h2>
    <p class="overlay-desc">
      Every stage's posterior overlaid on one axis (prior lightest → final
      stage darkest)
    </p>
    <div class="overlay-grid">
      <div class="overlay-cell">
        <p class="trace-label">R₀ — KDE per stage</p>
        <LineChart :series="stageOverlays.r0Series" :areas="stageOverlays.r0Areas" :height="240" x-label="R₀"
          y-label="density" :menu="false" tooltip-trigger="hover" tooltip-value-format="%.3f" />
      </div>
      <div class="overlay-cell">
        <p class="trace-label">Initial infections — posterior per stage</p>
        <BarChart :categories="stageOverlays.iiCategories" :series="stageOverlays.iiSeries" layout="overlay"
          :height="240" :menu="false" tooltip-trigger="hover" tooltip-value-format="%.4f" />
      </div>
    </div>
  </section>

  <section v-if="stageTraces.length">
    <h2>Per-stage parameter trace</h2>
    <table class="stage-summary">
      <thead>
        <tr>
          <th>Stage</th>
          <th class="num">Particles</th>
          <th class="num">Acceptance</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="row in summaryRows" :key="row.stage">
          <td>{{ row.stage }}</td>
          <td class="num">{{ row.particles }}</td>
          <td class="num">{{ row.acceptance }}</td>
        </tr>
      </tbody>
    </table>
    <div class="trace-grid">
      <div v-for="trace in stageTraces" :key="trace.stage" class="trace-cell">
        <h4>{{ trace.stageLabel }} (n={{ trace.n }})</h4>
        <div v-if="trace.trajectorySeries.length" class="trace-trajectory">
          <p class="trace-label">Trajectories (observed overlaid in teal)</p>
          <LineChart :series="trace.trajectorySeries" :height="160" x-label="Day" :y-label="valueLabel" :menu="false" />
        </div>
        <div class="trace-row">
          <div class="trace-mini">
            <p class="trace-label">R₀ (prior · posterior · KDE)</p>
            <BarChart :categories="trace.r0Bins.categories" :series="[
              {
                data: trace.r0Bins.prior,
                color: '#94a3b8',
                blendMode: 'multiply',
                legend: 'Prior',
              },
              {
                data: trace.r0Bins.posterior,
                color: '#2563eb',
                blendMode: 'multiply',
                legend: 'Posterior',
              },
            ]" layout="overlay" :summary-lines="[
              {
                x: trace.r0KdeOverlay.x,
                data: trace.r0KdeOverlay.data,
                color: '#dc2626',
                strokeWidth: 2,
                dots: false,
              },
            ]" :height="180" :menu="false" tooltip-trigger="hover" tooltip-value-format="%.4f" />
          </div>
          <div class="trace-mini">
            <!-- Initial-infections is discrete + frozen by the
                 perturbation kernel, so no KDE overlay. -->
            <p class="trace-label">Initial infections (prior · posterior)</p>
            <BarChart :categories="trace.iiBins.categories" :series="[
              {
                data: trace.iiBins.prior,
                color: '#94a3b8',
                blendMode: 'multiply',
                legend: 'Prior',
              },
              {
                data: trace.iiBins.posterior,
                color: '#2563eb',
                blendMode: 'multiply',
                legend: 'Posterior',
              },
            ]" layout="overlay" :height="180" :menu="false" tooltip-trigger="hover" tooltip-value-format="%.4f" />
          </div>
        </div>
      </div>
    </div>
  </section>
</template>

<!-- Non-scoped: the calibrate page needs more horizontal room than the
     SidebarLayout's default 1024px MainContent cap. Overrides the
     cfasim-ui rule (`.MainContent[data-v-*]{max-width:1024px}`). -->
<style>
.MainContent {
  max-width: 1400px !important;
}
</style>

<style scoped>
.sidebar-header {
  display: flex;
  flex-direction: column;
  gap: 0.5em;
}

.sidebar-header h2 {
  margin: 0;
}

.run-list {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.run-list__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: var(--font-size-sm, 0.875rem);
  color: var(--color-text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.run-list__empty {
  margin: 0.25rem 0;
  font-size: var(--font-size-sm, 0.875rem);
  color: var(--color-text-secondary);
}

.run-list__items {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md, 6px);
  overflow: hidden;
  background: var(--color-bg-0);
}

.run-list__item {
  display: flex;
  align-items: stretch;
  border-bottom: 1px solid var(--color-border);
}

.run-list__item:last-child {
  border-bottom: none;
}

.run-list__item--active {
  background: var(--color-bg-2);
}

.run-list__select {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  padding: 0.45rem 0.55rem;
  background: transparent;
  border: none;
  cursor: pointer;
  text-align: left;
  font: inherit;
  color: var(--color-text);
  /* Disable the OS double-click text selection on the name so the
     rename interaction doesn't highlight the row's text. */
  user-select: none;
}

.run-list__edit-form {
  flex: 1;
  display: flex;
  padding: 0.45rem 0.55rem;
}

.run-list__edit-input {
  flex: 1;
  background: var(--color-bg-1);
  color: var(--color-text);
  border: 1px solid var(--color-primary);
  border-radius: var(--radius-sm, 4px);
  padding: 0.25rem 0.4rem;
  font: inherit;
  font-size: var(--font-size-sm, 0.875rem);
  outline: none;
}

.run-list__edit-input--invalid {
  border-color: var(--color-error);
  background: var(--color-box-error-bg);
}

.run-list__select:hover {
  background: var(--color-bg-1);
}

.run-list__item--active .run-list__select:hover {
  /* Keep the active highlight when hovering the active row. */
  background: var(--color-bg-2);
}

.run-list__name {
  font-size: var(--font-size-sm, 0.875rem);
  color: var(--color-text);
}

.run-list__status {
  font-size: var(--font-size-xs, 0.75rem);
  color: var(--color-text-secondary);
}

.run-list__delete {
  width: 1.8rem;
  background: transparent;
  border: none;
  border-left: 1px solid var(--color-border);
  cursor: pointer;
  font-size: 1.1rem;
  color: var(--color-text-secondary);
  line-height: 1;
}

.run-list__delete:hover {
  color: var(--color-error);
  background: var(--color-box-error-bg);
}

.confirm-dialog {
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg, 8px);
  padding: 1rem 1.25rem;
  max-width: 24rem;
  background: var(--color-bg-0);
  color: var(--color-text);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
}

.confirm-dialog::backdrop {
  background: rgba(0, 0, 0, 0.5);
}

.confirm-dialog__title {
  margin: 0 0 0.5rem;
  font-size: var(--font-size-lg, 1.125rem);
  color: var(--color-text);
}

.confirm-dialog__body {
  margin: 0 0 1rem;
  font-size: var(--font-size-sm, 0.9rem);
  color: var(--color-text);
}

.confirm-dialog__actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
}

/* Reset the native fieldset chrome so it's a transparent wrapper whose
   only job is to cascade `disabled` to the param controls inside. */
.cal-fieldset {
  border: 0;
  margin: 0;
  padding: 0;
  min-inline-size: 0;
  display: contents;
}

.cal-locked-banner {
  margin: 0.75rem 0 0;
  padding: 0.5rem 0.75rem;
  font-size: var(--font-size-sm, 0.875rem);
  color: var(--color-text-muted, #64748b);
  background: var(--color-surface-muted, #f1f5f9);
  border: 1px solid var(--color-border);
  border-radius: 0.375rem;
}

.cal-section {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  padding-top: 0.75rem;
  border-top: 1px solid var(--color-border);
}

.cal-section h3 {
  margin: 0;
  font-size: var(--font-size-md, 1rem);
}

.cal-hint {
  margin: 0;
  font-size: var(--font-size-xs, 0.75rem);
  color: var(--color-text-secondary);
}

.cal-metric {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  margin-bottom: 1.5rem;
  max-width: 38rem;
}

.cal-metric__desc {
  margin: 0;
  font-size: var(--font-size-sm, 0.875rem);
  color: var(--color-text-secondary);
}

.cal-metric__locked {
  margin: 0;
  display: flex;
  gap: 0.4rem;
  align-items: baseline;
  font-size: var(--font-size-sm, 0.875rem);
}

.cal-target {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  margin-bottom: 1.5rem;
}

.cal-target__plot {
  margin: 0;
}

.obs-editor {
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
  align-items: flex-start;
}

.obs-editor__actions {
  display: flex;
  flex-wrap: wrap;
  gap: 0.4rem;
  align-items: center;
}

.obs-editor__file {
  display: none;
}

/* The editor body is a transparent (display:contents) fieldset; restore
   a column flow so the sheet + Add row button stack with a gap. */
.obs-editor__body.cal-fieldset {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  align-items: flex-start;
  width: 100%;
  max-width: 24rem;
}

.obs-sheet-scroll {
  width: 100%;
  max-height: 20rem;
  overflow-y: auto;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm, 4px);
  background: var(--color-bg-0);
  /* Firefox: thin scrollbar with a defined track so the gutter doesn't
     read as a stray white strip. */
  scrollbar-width: thin;
  scrollbar-color: var(--color-border) var(--color-bg-1);
}

/* WebKit/Blink: give the scrollbar gutter a track background, a divider
   from the cells, and a rounded thumb. */
.obs-sheet-scroll::-webkit-scrollbar {
  width: 12px;
}

.obs-sheet-scroll::-webkit-scrollbar-track {
  background: var(--color-bg-1);
  border-left: 1px solid var(--color-border);
}

.obs-sheet-scroll::-webkit-scrollbar-thumb {
  background: var(--color-border);
  border-radius: 6px;
  border: 3px solid var(--color-bg-1);
}

.obs-sheet-scroll::-webkit-scrollbar-thumb:hover {
  background: var(--color-text-secondary);
}

.obs-sheet {
  width: 100%;
  border-collapse: collapse;
  table-layout: fixed;
  font-size: var(--font-size-sm, 0.875rem);
  font-variant-numeric: tabular-nums;
}

.obs-sheet th {
  position: sticky;
  top: 0;
  z-index: 1;
  background: var(--color-bg-1);
  text-align: right;
  font-weight: 500;
  font-size: var(--font-size-xs, 0.75rem);
  color: var(--color-text-secondary);
  padding: 0.3rem 0.55rem;
  border-bottom: 1px solid var(--color-border);
}

.obs-sheet th:first-child {
  width: 6rem;
}

.obs-sheet th:last-child {
  width: 2.4rem;
}

/* Flush internal gridlines so the cells read as one continuous sheet. */
.obs-sheet th:not(:last-child),
.obs-sheet td:not(:last-child) {
  border-right: 1px solid var(--color-border);
}

.obs-sheet td {
  padding: 0;
  border-bottom: 1px solid var(--color-border);
}

.obs-sheet tbody tr:last-child td {
  border-bottom: none;
}

.obs-cell {
  width: 100%;
  box-sizing: border-box;
  border: none;
  background: transparent;
  color: var(--color-text);
  padding: 0.3rem 0.55rem;
  font: inherit;
  font-variant-numeric: tabular-nums;
  text-align: right;
  appearance: textfield;
  -moz-appearance: textfield;
}

/* Drop the native spin buttons — they crowd the right-aligned numbers
   and break the flush spreadsheet look. */
.obs-cell::-webkit-outer-spin-button,
.obs-cell::-webkit-inner-spin-button {
  -webkit-appearance: none;
  margin: 0;
}

.obs-cell:focus {
  outline: 2px solid var(--color-primary);
  outline-offset: -2px;
  background: var(--color-bg-0);
}

.obs-sheet__rm {
  text-align: center;
}

.obs-rm {
  border: none;
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
  font-size: 1rem;
  line-height: 1;
  padding: 0.25rem 0.45rem;
}

.obs-rm:hover {
  color: var(--color-error);
}

.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}

.sidebar-controls {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5em;
  margin-top: 0.75rem;
}

.status {
  color: var(--color-text-secondary);
}

.error {
  color: var(--color-error);
}

.stage-summary {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--font-size-sm, 0.9rem);
  margin: 0.5rem 0;
}

.stage-summary th,
.stage-summary td {
  text-align: left;
  padding: 0.35rem 0.5rem;
  border-bottom: 1px solid var(--color-border);
}

.stage-summary th.num,
.stage-summary td.num {
  text-align: right;
}

.overlay-section {
  margin-bottom: 1.5rem;
}

.overlay-desc {
  margin: 0 0 0.75rem;
  font-size: var(--font-size-sm, 0.875rem);
  color: var(--color-text-secondary);
}

.overlay-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 1rem;
}

.overlay-cell {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md, 6px);
  padding: 0.75rem;
  background: var(--color-bg-0);
}

.trace-grid {
  display: flex;
  flex-direction: column;
  gap: 1rem;
  margin-top: 1rem;
}

.trace-cell {
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md, 6px);
  padding: 0.75rem;
  background: var(--color-bg-0);
}

.trace-cell h4 {
  margin: 0 0 0.5rem;
  font-size: var(--font-size-sm, 0.9rem);
  color: var(--color-text);
}

.trace-trajectory {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  margin-bottom: 0.75rem;
}

.trace-row {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 1rem;
}

.trace-mini {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.trace-label {
  margin: 0;
  font-size: var(--font-size-xs, 0.75rem);
  color: var(--color-text-secondary);
}
</style>
