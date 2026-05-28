// IndexedDB persistence for ABC-SMC calibration runs. Wraps `idb` so the
// rest of the app can read/write runs with a Promise-based API.
//
// Schema (db: "ixa-calibration", v1):
//   - `runs` store, key = runId (string, ULID). Value = StoredRun.
//   - `particles` store, key = [runId, stage, particleIdx]. Value = StoredParticle.

import { openDB, type IDBPDatabase } from "idb";
import type { CalibrationConfig, Particle } from "./calibration";

// IndexedDB uses the structured-clone algorithm, which throws
// `DataCloneError` on any Vue Proxy. Callers may hand us reactive values
// (refs around arrays, shallowReactive objects); JSON round-trip strips
// every wrapper and leaves only plain data, which all of our types are.
function plain<T>(v: T): T {
  return JSON.parse(JSON.stringify(v));
}

const DB_NAME = "ixa-calibration";
const DB_VERSION = 1;
const RUNS_STORE = "runs";
const PARTICLES_STORE = "particles";

/// Data-shape version stamped on every stored row. Bump this whenever
/// `StoredRun.config`, `StoredParticle`, or any other persisted shape
/// changes in a non-backwards-compatible way. `loadRun` / `listRuns`
/// skip rows with a different schemaVersion and the UI surfaces them as
/// stale (the user can discard via the delete dialog).
///
/// Distinct from `DB_VERSION` above: `DB_VERSION` controls the IDB
/// `upgrade` callback (object stores + indexes). `SCHEMA_VERSION`
/// describes the application-level shape of the bytes inside those
/// stores — that can change without changing the store layout (e.g.
/// renaming a config field).
// v2: StoredParticle gained a `seed` field for the seed-observation
//     diagnostic. Older v1 rows don't carry it, so loadRun's schema
//     check rejects them — the user can discard via the delete dialog.
export const SCHEMA_VERSION = 2;

export type RunStatus = "idle" | "running" | "paused" | "complete" | "error";

export interface StoredRun {
  id: string;
  name: string;
  config: CalibrationConfig;
  createdAt: number;
  updatedAt: number;
  status: RunStatus;
  /// Generation index currently being filled. 0 = sample-from-prior pass;
  /// 1..=config.stages.length = perturbation stages.
  currentStage: number;
  /// Per-stage acceptance metrics, length grows as stages complete.
  acceptance: { nAttempts: number; nAccepted: number }[];
  /// Last error message if status === "error". Undefined otherwise.
  errorMessage?: string;
  /// `SCHEMA_VERSION` at the time this row was written. Used to fence
  /// off rows from older incompatible code. Optional in the type so
  /// pre-versioning rows still deserialize; reads treat undefined as 0.
  schemaVersion?: number;
}

export interface StoredParticle {
  runId: string;
  stage: number;
  particleIdx: number;
  r0: number;
  initialInfections: number;
  weight: number;
  distance: number;
  /// Per-particle daily-incidence trajectory; length = floor(maxTime).
  trajectory: number[];
  /// Seed key as `"<hi32>-<lo32>"` (see Particle.seed). Optional so the
  /// type tolerates pre-v2 rows during read-back; new writes always
  /// include it.
  seed?: string;
  /// See `StoredRun.schemaVersion`.
  schemaVersion?: number;
}

let dbPromise: Promise<IDBPDatabase> | null = null;

/// Test-only: close the cached connection and clear the promise so the
/// next `db()` call re-opens fresh. Needed because vitest tests want to
/// `indexedDB.deleteDatabase("ixa-calibration")` between cases, and IDB
/// blocks that delete while a connection is open.
export async function _resetDbForTesting(): Promise<void> {
  if (dbPromise) {
    try {
      const d = await dbPromise;
      d.close();
    } catch {
      // ignore — connection failed to open in the first place.
    }
    dbPromise = null;
  }
}

function db(): Promise<IDBPDatabase> {
  if (!dbPromise) {
    dbPromise = openDB(DB_NAME, DB_VERSION, {
      upgrade(d) {
        if (!d.objectStoreNames.contains(RUNS_STORE)) {
          d.createObjectStore(RUNS_STORE, { keyPath: "id" });
        }
        if (!d.objectStoreNames.contains(PARTICLES_STORE)) {
          const store = d.createObjectStore(PARTICLES_STORE, {
            keyPath: ["runId", "stage", "particleIdx"],
          });
          store.createIndex("by_run", "runId");
          store.createIndex("by_run_stage", ["runId", "stage"]);
        }
      },
    });
  }
  return dbPromise;
}

// Short ULID-ish id: 8 hex chars of timestamp + 6 hex chars of randomness.
// Sortable by creation time; no external dep.
function newRunId(): string {
  const ts = Date.now().toString(16).padStart(11, "0");
  const rand = Math.floor(Math.random() * 0xffffff)
    .toString(16)
    .padStart(6, "0");
  return `${ts}${rand}`;
}

export async function createRun(
  config: CalibrationConfig,
  name: string,
): Promise<string> {
  const id = newRunId();
  const now = Date.now();
  const run: StoredRun = {
    id,
    name,
    config: plain(config),
    createdAt: now,
    updatedAt: now,
    status: "idle",
    currentStage: 0,
    acceptance: [],
    schemaVersion: SCHEMA_VERSION,
  };
  const d = await db();
  await d.put(RUNS_STORE, run);
  return id;
}

export async function saveBatch(
  runId: string,
  stage: number,
  startIdx: number,
  particles: Particle[],
): Promise<void> {
  const d = await db();
  const tx = d.transaction(PARTICLES_STORE, "readwrite");
  for (let i = 0; i < particles.length; i++) {
    // `plain()` is the storage layer's one source of truth for proxy
    // stripping; keeping every put on the same code path means adding a
    // field to `StoredParticle` doesn't require a parallel edit here.
    tx.store.put(
      plain({
        runId,
        stage,
        particleIdx: startIdx + i,
        ...particles[i],
        schemaVersion: SCHEMA_VERSION,
      }),
    );
  }
  await tx.done;
}

export async function loadRun(
  runId: string,
): Promise<
  | {
      run: StoredRun;
      particlesByStage: Particle[][];
    }
  | null
> {
  const d = await db();
  const run = (await d.get(RUNS_STORE, runId)) as StoredRun | undefined;
  if (!run) return null;
  if ((run.schemaVersion ?? 0) !== SCHEMA_VERSION) {
    // Wrong-shape data from an older build. Return null so the page
    // treats it as "no such run"; the row stays in IDB so the user
    // can delete it (the list still shows it via listRuns's separate
    // schema check).
    console.warn(
      `[calibrationStorage] skipping run ${runId}: schemaVersion=${run.schemaVersion} (need ${SCHEMA_VERSION})`,
    );
    return null;
  }
  const all = (await d.getAllFromIndex(
    PARTICLES_STORE,
    "by_run",
    runId,
  )) as StoredParticle[];
  // Group by stage; keep particle order via particleIdx.
  const nStages = run.config.stages.length + 1; // +1 for stage 0 prior sample
  const particlesByStage: Particle[][] = Array.from(
    { length: nStages },
    () => [],
  );
  all.sort((a, b) =>
    a.stage !== b.stage ? a.stage - b.stage : a.particleIdx - b.particleIdx,
  );
  for (const sp of all) {
    if (sp.stage < 0 || sp.stage >= nStages) continue;
    particlesByStage[sp.stage].push({
      r0: sp.r0,
      initialInfections: sp.initialInfections,
      weight: sp.weight,
      distance: sp.distance,
      trajectory: sp.trajectory ?? [],
      seed: sp.seed,
    });
  }
  return { run, particlesByStage };
}

export async function listRuns(): Promise<StoredRun[]> {
  const d = await db();
  const all = (await d.getAll(RUNS_STORE)) as StoredRun[];
  // Surface all runs (including stale-schema ones) so the user can see
  // and delete them; the caller can inspect `schemaVersion` to flag
  // stale rows in the UI.
  return all.sort((a, b) => b.createdAt - a.createdAt);
}

/// True iff `run` was written by the current code's data shape.
export function isCurrentSchema(run: StoredRun): boolean {
  return (run.schemaVersion ?? 0) === SCHEMA_VERSION;
}

/// `errorMessage: null` means "clear this field". We can't use
/// `undefined` because `JSON.stringify` drops undefined keys, so
/// `plain(patch)` would lose the key and the spread couldn't override
/// the stored value.
export type StatusPatch = Partial<
  Pick<StoredRun, "status" | "currentStage" | "acceptance" | "name">
> & { errorMessage?: string | null };

export async function setStatus(
  runId: string,
  patch: StatusPatch,
): Promise<void> {
  const d = await db();
  const tx = d.transaction(RUNS_STORE, "readwrite");
  const existing = (await tx.store.get(runId)) as StoredRun | undefined;
  if (!existing) {
    await tx.done;
    return;
  }
  const updated: StoredRun = {
    ...existing,
    ...plain(patch),
    updatedAt: Date.now(),
  };
  if ("errorMessage" in patch) {
    // Explicit clear when caller passed null. Otherwise plain(patch)
    // already set the field to the string.
    if (patch.errorMessage === null) updated.errorMessage = undefined;
  }
  await tx.store.put(updated);
  await tx.done;
}

/// Replace a run's `config` (typically called when the user adjusts the
/// sidebar before clicking Start on a freshly-created idle run). Keeps
/// `name`, `status`, `currentStage`, `acceptance` untouched.
export async function updateConfig(
  runId: string,
  config: CalibrationConfig,
): Promise<void> {
  const d = await db();
  const tx = d.transaction(RUNS_STORE, "readwrite");
  const existing = (await tx.store.get(runId)) as StoredRun | undefined;
  if (!existing) {
    await tx.done;
    return;
  }
  const updated: StoredRun = {
    ...existing,
    config: plain(config),
    updatedAt: Date.now(),
  };
  await tx.store.put(updated);
  await tx.done;
}

export async function deleteRun(runId: string): Promise<void> {
  const d = await db();
  const tx = d.transaction(
    [RUNS_STORE, PARTICLES_STORE],
    "readwrite",
  );
  tx.objectStore(RUNS_STORE).delete(runId);
  const particles = tx.objectStore(PARTICLES_STORE);
  const keys = await particles.index("by_run").getAllKeys(runId);
  for (const k of keys) particles.delete(k);
  await tx.done;
}
