import { describe, it, expect, beforeEach } from "vitest";
import "fake-indexeddb/auto";

import {
  createRun,
  saveBatch,
  loadRun,
  listRuns,
  setStatus,
  deleteRun,
  isCurrentSchema,
  SCHEMA_VERSION,
  _resetDbForTesting,
  type StoredRun,
} from "./calibrationStorage";
import { defaultConfig, type Particle } from "./calibration";

function particle(r0: number, ii: number, dist = 1, weight = 1): Particle {
  return {
    r0,
    initialInfections: ii,
    weight,
    distance: dist,
    trajectory: [0, ii, ii * 2, ii * 3],
  };
}

beforeEach(async () => {
  // Close the module's cached connection first; otherwise the delete
  // request below is `onblocked` and the data persists across tests.
  await _resetDbForTesting();
  await new Promise<void>((resolve) => {
    const req = indexedDB.deleteDatabase("ixa-calibration");
    req.onsuccess = () => resolve();
    req.onerror = () => resolve();
    req.onblocked = () => resolve();
  });
});

describe("createRun + listRuns + loadRun", () => {
  it("creates a run and lists it", async () => {
    const id = await createRun(defaultConfig(), "first");
    expect(typeof id).toBe("string");
    expect(id.length).toBeGreaterThan(8);
    const runs = await listRuns();
    expect(runs.some((r) => r.id === id && r.name === "first")).toBe(true);
  });

  it("returns null for an unknown runId", async () => {
    expect(await loadRun("nonexistent")).toBeNull();
  });

  it("loads a run with the right config + empty particles", async () => {
    const id = await createRun(defaultConfig(), "second");
    const loaded = await loadRun(id);
    expect(loaded).not.toBeNull();
    expect(loaded!.run.name).toBe("second");
    expect(loaded!.run.status).toBe("idle");
    expect(loaded!.run.currentStage).toBe(0);
    // 1 prior stage + config.stages.length perturbation stages = total slots
    const expectedSlots = loaded!.run.config.stages.length + 1;
    expect(loaded!.particlesByStage.length).toBe(expectedSlots);
    expect(loaded!.particlesByStage.every((s) => s.length === 0)).toBe(true);
  });

  it("ignores a Vue-Proxy-like config (JSON round-trip strips wrappers)", async () => {
    const config = defaultConfig();
    // Synthesize a "proxy" stand-in: a getter that throws when structured-cloned
    // would be the realistic case, but JSON.stringify reads it normally.
    const wrapped = JSON.parse(JSON.stringify(config));
    const id = await createRun(wrapped, "third");
    const loaded = await loadRun(id);
    expect(loaded!.run.config).toEqual(config);
  });
});

describe("saveBatch + loadRun round-trip", () => {
  it("persists particles and restores them grouped by stage + ordered by particleIdx", async () => {
    const id = await createRun(defaultConfig(), "with-particles");
    const stage0 = [particle(1, 5), particle(2, 6), particle(3, 7)];
    const stage1 = [particle(1.5, 5), particle(2.5, 6)];
    await saveBatch(id, 0, 0, stage0);
    await saveBatch(id, 1, 0, stage1);
    const loaded = await loadRun(id);
    expect(loaded!.particlesByStage[0]).toHaveLength(3);
    expect(loaded!.particlesByStage[1]).toHaveLength(2);
    expect(loaded!.particlesByStage[0][2].r0).toBe(3);
    expect(loaded!.particlesByStage[1][1].initialInfections).toBe(6);
    // Trajectories survive round-trip.
    expect(loaded!.particlesByStage[0][0].trajectory).toEqual([0, 5, 10, 15]);
  });

  it("appends across multiple saveBatch calls into the same stage", async () => {
    const id = await createRun(defaultConfig(), "split-batches");
    await saveBatch(id, 0, 0, [particle(1, 1), particle(2, 2)]);
    await saveBatch(id, 0, 2, [particle(3, 3)]);
    const loaded = await loadRun(id);
    expect(loaded!.particlesByStage[0].map((p) => p.r0)).toEqual([1, 2, 3]);
  });

  it("preserves empty trajectory arrays through the round-trip", async () => {
    const id = await createRun(defaultConfig(), "empty-traj");
    await saveBatch(id, 0, 0, [{ ...particle(1, 1), trajectory: [] }]);
    const loaded = await loadRun(id);
    expect(loaded!.particlesByStage[0][0].trajectory).toEqual([]);
  });
});

describe("setStatus", () => {
  it("patches fields without overwriting unrelated ones", async () => {
    const id = await createRun(defaultConfig(), "patch");
    await setStatus(id, { status: "running", currentStage: 2 });
    const loaded = await loadRun(id);
    expect(loaded!.run.status).toBe("running");
    expect(loaded!.run.currentStage).toBe(2);
    expect(loaded!.run.name).toBe("patch");
  });

  it("ignores setStatus on a missing run (no throw, no insertion)", async () => {
    await setStatus("does-not-exist", { status: "complete" });
    const runs = await listRuns();
    expect(runs.find((r) => r.id === "does-not-exist")).toBeUndefined();
  });

  it("clears errorMessage when passed null (undefined would be lost in JSON round-trip)", async () => {
    const id = await createRun(defaultConfig(), "clearable");
    await setStatus(id, { status: "error", errorMessage: "boom" });
    expect((await loadRun(id))!.run.errorMessage).toBe("boom");
    // The previous attempt was a bug: undefined gets dropped by JSON.stringify
    // and the stale message stays. Null is the documented "clear" sentinel.
    await setStatus(id, { status: "running", errorMessage: null });
    expect((await loadRun(id))!.run.errorMessage).toBeUndefined();
  });
});

describe("schema versioning", () => {
  it("stamps SCHEMA_VERSION on newly created runs", async () => {
    const id = await createRun(defaultConfig(), "stamped");
    const loaded = await loadRun(id);
    expect(loaded!.run.schemaVersion).toBe(SCHEMA_VERSION);
    expect(isCurrentSchema(loaded!.run)).toBe(true);
  });

  it("returns null from loadRun when the stored schemaVersion doesn't match", async () => {
    const id = await createRun(defaultConfig(), "stale");
    // Poke the IDB row directly to simulate a row written by older code.
    await new Promise<void>((resolve, reject) => {
      const open = indexedDB.open("ixa-calibration");
      open.onsuccess = () => {
        const dbConn = open.result;
        const tx = dbConn.transaction("runs", "readwrite");
        const store = tx.objectStore("runs");
        const getReq = store.get(id);
        getReq.onsuccess = () => {
          const row = getReq.result as StoredRun;
          row.schemaVersion = 0; // pretend an old write
          store.put(row);
        };
        tx.oncomplete = () => {
          dbConn.close();
          resolve();
        };
        tx.onerror = () => reject(tx.error);
      };
      open.onerror = () => reject(open.error);
    });
    await _resetDbForTesting();
    expect(await loadRun(id)).toBeNull();
    // listRuns still shows it, so the user can delete via the UI.
    const list = await listRuns();
    expect(list.find((r) => r.id === id)).toBeDefined();
    expect(isCurrentSchema(list.find((r) => r.id === id)!)).toBe(false);
  });
});

describe("deleteRun", () => {
  it("removes the run + all its particles", async () => {
    const id = await createRun(defaultConfig(), "to-delete");
    await saveBatch(id, 0, 0, [particle(1, 1), particle(2, 2)]);
    await deleteRun(id);
    expect(await loadRun(id)).toBeNull();
    expect((await listRuns()).find((r) => r.id === id)).toBeUndefined();
  });

  it("only deletes the targeted run's particles", async () => {
    const a = await createRun(defaultConfig(), "keep");
    const b = await createRun(defaultConfig(), "drop");
    await saveBatch(a, 0, 0, [particle(1, 1)]);
    await saveBatch(b, 0, 0, [particle(2, 2)]);
    await deleteRun(b);
    const aLoaded = await loadRun(a);
    expect(aLoaded!.particlesByStage[0]).toHaveLength(1);
    expect(aLoaded!.particlesByStage[0][0].r0).toBe(1);
  });
});
