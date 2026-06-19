import { describe, it, expect } from "vitest";
import {
  DEFAULT_FACEMASK,
  DEFAULT_ANTIVIRAL,
  remainingInfectiousness,
} from "./modifiers";

describe("modifiers", () => {
  it("default configs are within the valid Rust ranges", () => {
    for (const v of [DEFAULT_FACEMASK.coverage, DEFAULT_FACEMASK.effectiveness]) {
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThanOrEqual(1);
    }
    for (const v of [DEFAULT_ANTIVIRAL.coverage, DEFAULT_ANTIVIRAL.efficacy]) {
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThanOrEqual(1);
    }
    expect(DEFAULT_ANTIVIRAL.delay).toBeGreaterThanOrEqual(0);
  });

  it("remainingInfectiousness is 1 - reduction", () => {
    expect(remainingInfectiousness(0)).toBe(1);
    expect(remainingInfectiousness(0.7)).toBeCloseTo(0.3, 12);
    expect(remainingInfectiousness(1)).toBe(0);
  });

  it("remainingInfectiousness clamps out-of-range / non-finite inputs", () => {
    expect(remainingInfectiousness(1.5)).toBe(0);
    expect(remainingInfectiousness(-0.5)).toBe(1);
    expect(remainingInfectiousness(NaN)).toBe(1);
  });
});
