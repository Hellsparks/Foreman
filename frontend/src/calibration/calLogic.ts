/** Small pure helpers for the Calibration tab (the maths itself runs in Rust). */
import type { Plane } from "../api/types/Plane";
import type { PlaneMeasure } from "../api/types/PlaneMeasure";
import type { PlaneSkew } from "../api/types/PlaneSkew";
import type { ShrinkageInput } from "../api/types/ShrinkageInput";

export type PlaneFields = Record<Plane, [string, string, string]>;
export type Axis = "x" | "y" | "z";
export type AxisFields = Record<Axis, { nominal: string; measured: string }>;

export const EMPTY_PLANES: PlaneFields = { XY: ["", "", ""], XZ: ["", "", ""], YZ: ["", "", ""] };
export const EMPTY_AXES: AxisFields = {
  x: { nominal: "100", measured: "" },
  y: { nominal: "100", measured: "" },
  z: { nominal: "100", measured: "" },
};

function num(s: string): number | null {
  const v = Number(s.trim().replace(",", "."));
  return s.trim() !== "" && Number.isFinite(v) ? v : null;
}

/** Planes with all three lengths typed; XY always first. `null` until XY is complete. */
export function planesFrom(f: PlaneFields, more: boolean): PlaneMeasure[] | null {
  const out: PlaneMeasure[] = [];
  for (const plane of (more ? ["XY", "XZ", "YZ"] : ["XY"]) as Plane[]) {
    const [ac, bd, ad] = f[plane].map(num);
    if (ac == null || bd == null || ad == null) {
      if (plane === "XY") return null;
      continue;
    }
    out.push({ plane, ac, bd, ad });
  }
  return out;
}

/** "−0.97°" with a real minus sign, "+0.31°". */
export function deg(d: number): string {
  const r = Math.round(d * 100) / 100;
  return `${r < 0 ? "−" : "+"}${Math.abs(r).toFixed(2)}°`;
}

export function skewSummary(planes: PlaneSkew[]): string {
  return planes.map((p) => `${p.plane} ${deg(p.degrees)}`).join(" · ");
}

/** Shrinkage input from the per-axis fields; axes without a measurement are left out. */
export function shrinkInput(a: AxisFields): ShrinkageInput | null {
  const span = (k: Axis) => {
    const n = num(a[k].nominal);
    const m = num(a[k].measured);
    return n != null && m != null && n > 0 && m > 0 ? [{ nominal: n, readings: [m] }] : [];
  };
  const input: ShrinkageInput = { x: span("x"), y: span("y"), z: span("z"), cal_err: null };
  return input.x.length + input.y.length + input.z.length > 0 ? input : null;
}

/** Back from a stored input to the fields (re-measure). */
export function axesFrom(input: ShrinkageInput): AxisFields {
  const one = (s: ShrinkageInput["x"]) => ({
    nominal: s[0] ? String(s[0].nominal) : "100",
    measured: s[0]?.readings[0] != null ? String(s[0].readings[0]) : "",
  });
  return { x: one(input.x), y: one(input.y), z: one(input.z) };
}

export function pct(v: number | null | undefined): string {
  return v == null ? "—" : `${v.toFixed(3)} %`;
}

export function shortDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}
