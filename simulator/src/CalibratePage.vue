<script setup lang="ts">
import { shallowReactive, ref, computed, onMounted } from "vue";
import { useRouter, useRoute } from "vue-router";
import { runWasm } from "cfasim-ui/wasm";
import type { ModelOutput } from "cfasim-ui/shared";
import {
  NumberInput,
  Button,
  SelectBox,
  TextInput,
} from "cfasim-ui/components";
import type { SelectOption } from "cfasim-ui/components";
// Native <dialog> element is used for the delete-confirmation modal —
// no extra component dependency.
import { BarChart, LineChart } from "cfasim-ui/charts";
import RateEditor from "./components/RateEditor.vue";
import PriorEditor from "./components/PriorEditor.vue";
import {
  defaultConfig,
  weightedHistogram,
  totalWeight,
  parseTargetCsv,
  acceptanceRatio,
  withR0,
  cumulativeToIncidence,
  type CalibrationConfig,
  type Particle,
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
  priors: seed.priors,
  stagesText: seed.stages.join(","),
  nParticles: seed.nParticles,
  batchSize: seed.batchSize,
  seed: seed.seed,
  targetMode: (seed.target.mode === "synthetic" ? "synthetic" : "csv") as
    | "synthetic"
    | "csv",
  truthR0:
    seed.target.mode === "synthetic" ? seed.target.truthR0 : 1.5,
  truthInitialInfections:
    seed.target.mode === "synthetic"
      ? seed.target.truthInitialInfections
      : 10,
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
const observed = ref<number[]>([]);

const targetModeOptions: SelectOption[] = [
  { value: "synthetic", label: "Synthetic from \"true\" params" },
  { value: "csv", label: "Uploaded CSV" },
];

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
  priors: params.priors,
  stages: parseStages(params.stagesText),
  nParticles: params.nParticles,
  batchSize: params.batchSize,
  seed: params.seed,
  observed: observed.value,
  target:
    params.targetMode === "synthetic"
      ? {
          mode: "synthetic",
          truthInitialInfections: params.truthInitialInfections,
          truthR0: params.truthR0,
        }
      : { mode: "csv", filename: params.csvFilename },
}));

const runner = useCalibration();

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
    setParam("priors", c.priors);
    setParam("stagesText", c.stages.join(","));
    setParam("nParticles", c.nParticles);
    setParam("batchSize", c.batchSize);
    setParam("seed", c.seed);
    observed.value = c.observed;
    if (c.target.mode === "synthetic") {
      setParam("targetMode", "synthetic");
      setParam("truthR0", c.target.truthR0);
      setParam("truthInitialInfections", c.target.truthInitialInfections);
    } else {
      setParam("targetMode", "csv");
      setParam("csvFilename", c.target.filename);
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
    infectionRate: withR0(params.infectionRate, params.truthR0),
    population: params.population,
    initialInfections: params.truthInitialInfections,
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

async function regenerateObservedPreview() {
  if (params.targetMode !== "synthetic") return;
  observed.value = await generateSyntheticObserved();
}

async function onUploadCsv(ev: Event) {
  const input = ev.target as HTMLInputElement;
  const f = input.files?.[0];
  if (!f) return;
  const text = await f.text();
  try {
    observed.value = parseTargetCsv(text);
    setParam("csvFilename", f.name);
  } catch (e) {
    alert(`CSV parse failed: ${e instanceof Error ? e.message : String(e)}`);
  }
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
    if (params.targetMode === "synthetic") {
      observed.value = await generateSyntheticObserved();
    }
    if (observed.value.length === 0) {
      errorMessage.value =
        params.targetMode === "csv"
          ? "Upload a CSV before starting."
          : "Synthetic data was empty.";
      return;
    }
    if (observed.value.length !== Math.floor(params.maxTime)) {
      errorMessage.value = `Observed data length (${observed.value.length}) must equal floor(maxTime) (${Math.floor(params.maxTime)}).`;
      return;
    }
    // If no run is selected (rare — "+ New" auto-creates one), mint
    // one now so the rest of the flow is uniform.
    if (!runner.runId.value) {
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

interface StageTrace {
  stage: number;
  stageLabel: string;
  n: number;
  acceptance: number | null;
  r0Bins: { categories: string[]; data: number[] };
  iiBins: { categories: string[]; data: number[] };
  trajectorySeries: TrajectorySeries[];
}

const stageTraces = computed<StageTrace[]>(() => {
  const stages = runner.particlesByStage.value;
  const acc = runner.acceptance.value;
  const out: StageTrace[] = [];
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
    const a = acc[s];
    // Particle trajectories as faint gray lines; observed (if loaded) as
    // a bolder teal overlay so the eye can compare.
    const trajX = observed.value.length
      ? observed.value.map((_, i) => i + 1)
      : ps[0].trajectory.map((_, i) => i + 1);
    const trajectorySeries: TrajectorySeries[] = ps
      .filter((p) => p.trajectory.length > 0)
      .map((p) => ({
        x: trajX.slice(0, p.trajectory.length),
        data: p.trajectory,
        color: "rgba(100, 116, 139, 0.35)",
        strokeWidth: 1,
        dots: false,
      }));
    if (observed.value.length) {
      trajectorySeries.push({
        x: trajX,
        data: observed.value,
        color: "#14b8a6",
        strokeWidth: 2.5,
        dots: false,
      });
    }
    out.push({
      stage: s,
      stageLabel: s === 0 ? "Prior (∞)" : `Stage ${s}`,
      n: ps.length,
      acceptance: a ? acceptanceRatio(a.nAccepted, a.nAttempts) : null,
      r0Bins: {
        categories: r0Bins.map((b) => b.center.toFixed(2)),
        data: r0Bins.map((b) => b.weight),
      },
      iiBins: {
        categories: iiBins.map((b) => b.center.toFixed(0)),
        data: iiBins.map((b) => b.weight),
      },
      trajectorySeries,
    });
  }
  return out;
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
    acceptance:
      t.acceptance === null
        ? "—"
        : `${(t.acceptance * 100).toFixed(1)}%`,
  })),
);

const observedSeries = computed(() => {
  if (observed.value.length === 0) return [];
  return [
    {
      x: observed.value.map((_, i) => i + 1),
      data: observed.value,
      color: "#0f766e",
      strokeWidth: 2,
      dots: false,
    },
  ];
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
          <li
            v-for="r in savedRuns"
            :key="r.id"
            class="run-list__item"
            :class="{ 'run-list__item--active': r.id === selectedRunId }"
          >
            <form
              v-if="editingRunId === r.id"
              class="run-list__edit-form"
              @submit.prevent="commitEdit"
            >
              <input
                v-model="editingName"
                class="run-list__edit-input"
                :class="{ 'run-list__edit-input--invalid': editingInvalid }"
                :aria-label="`Rename ${r.name}`"
                :title="editingInvalid ? 'Name cannot be empty' : undefined"
                autofocus
                @keydown.escape.prevent="cancelEdit"
                @input="editingInvalid = false"
                @blur="commitEdit"
              />
            </form>
            <button
              v-else
              type="button"
              class="run-list__select"
              :title="`${r.name} · ${liveStatus(r)} (double-click to rename)`"
              @click="onSelectRun(r.id)"
              @dblclick="startEditing(r)"
            >
              <span class="run-list__name">{{ r.name }}</span>
              <span class="run-list__status">{{ liveStatus(r) }}</span>
            </button>
            <button
              v-if="editingRunId !== r.id"
              type="button"
              class="run-list__delete"
              :title="`Delete ${r.name}`"
              :aria-label="`Delete ${r.name}`"
              @click="requestDelete(r)"
            >
              ×
            </button>
          </li>
        </ul>
      </div>
    </div>

    <section class="cal-section">
      <h3>Model</h3>
      <RateEditor v-model="params.infectionRate" />
      <NumberInput v-model="params.population" label="Population" :min="100" />
      <NumberInput v-model="params.maxTime" label="Max time (days)" :min="1" />
    </section>

    <section class="cal-section">
      <h3>Target data</h3>
      <SelectBox
        label="Source"
        :options="targetModeOptions"
        :model-value="params.targetMode"
        @update:model-value="(v) => setParam('targetMode', String(v) as 'synthetic' | 'csv')"
      />
      <template v-if="params.targetMode === 'synthetic'">
        <NumberInput
          v-model="params.truthR0"
          label="True R₀"
          :min="0.1"
          :step="0.1"
        />
        <NumberInput
          v-model="params.truthInitialInfections"
          label="True initial infections"
          :min="1"
        />
        <Button variant="secondary" @click="regenerateObservedPreview">
          Regenerate preview
        </Button>
      </template>
      <template v-else>
        <label class="cal-upload">
          <span>Upload CSV (time,incident_cases)</span>
          <input type="file" accept=".csv,text/csv" @change="onUploadCsv" />
        </label>
        <p v-if="params.csvFilename" class="cal-csv-name">
          {{ params.csvFilename }} — {{ observed.length }} day(s)
        </p>
      </template>
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
      <TextInput
        v-model="params.stagesText"
        label="Error schedule (comma-separated, 0 < r < 1)"
      />
    </section>

    <div class="sidebar-controls">
      <Button
        @click="startRun"
        :disabled="runner.status.value === 'running'"
      >
        Start new run
      </Button>
      <Button
        v-if="runner.status.value === 'running'"
        variant="secondary"
        @click="pauseRun"
      >
        Pause
      </Button>
      <Button
        v-else-if="runner.status.value === 'paused'"
        variant="secondary"
        @click="resumeRun"
      >
        Resume
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
    Calibrates the transmission model to observed daily-incidence data
    via ABC-SMC. Pick priors, set an error schedule, then watch the
    posterior tighten across stages. Runs persist in your browser, so
    if even if you close the tab, you can resume later.
  </p>
  <p v-if="errorMessage" class="error">{{ errorMessage }}</p>
  <p class="status">{{ statusLabel }}</p>

  <section v-if="observed.length">
    <h2>Observed data</h2>
    <LineChart
      :series="observedSeries"
      :height="180"
      x-label="Day"
      y-label="Incident cases"
      :menu="false"
      tooltip-trigger="hover"
    />
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
      <div
        v-for="trace in stageTraces"
        :key="trace.stage"
        class="trace-cell"
      >
        <h4>{{ trace.stageLabel }} (n={{ trace.n }})</h4>
        <div v-if="trace.trajectorySeries.length" class="trace-trajectory">
          <p class="trace-label">Trajectories (observed overlaid in teal)</p>
          <LineChart
            :series="trace.trajectorySeries"
            :height="160"
            x-label="Day"
            y-label="Incident cases"
            :menu="false"
          />
        </div>
        <div class="trace-row">
          <div class="trace-mini">
            <p class="trace-label">R₀</p>
            <BarChart
              :categories="trace.r0Bins.categories"
              :data="trace.r0Bins.data"
              :height="120"
              :menu="false"
            />
          </div>
          <div class="trace-mini">
            <p class="trace-label">Initial infections</p>
            <BarChart
              :categories="trace.iiBins.categories"
              :data="trace.iiBins.data"
              :height="120"
              :menu="false"
            />
          </div>
        </div>
      </div>
    </div>
  </section>
</template>

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
.cal-upload {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  font-size: var(--font-size-sm, 0.875rem);
}
.cal-csv-name {
  margin: 0;
  font-size: var(--font-size-xs, 0.75rem);
  color: var(--color-text-secondary);
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
  grid-template-columns: 1fr 1fr;
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
