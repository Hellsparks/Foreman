/**
 * Loose-file projects (task 5.2): files' filament slots grouped into shared colour
 * slots. Same material + colour across files → one slot; the user can give a
 * filament its own slot (split) or move it into another slot (merge).
 */
import type { LibraryEntry } from "../api/library";
import type { FilamentSlot } from "../api/types/FilamentSlot";
import type { SlotKind } from "../api/types/SlotKind";

export interface PickedFile {
  file_id: number;
  name: string;
  slot_kind: SlotKind;
  slots: FilamentSlot[];
  /** Copies per set. */
  copies: number;
}

export interface Member {
  file_id: number;
  file: string;
  slot: FilamentSlot;
  copies: number;
}

export interface Group {
  key: string;
  material: string | null;
  /** `RRGGBB`, no `#`. */
  hex: string | null;
  members: Member[];
  /** Grams for one set (copies included). */
  perSet: number;
}

/** Override per file filament: `"fileId:slotIndex"` → slot key. */
export type Assign = Record<string, string>;

export const memberId = (fileId: number, slot: number) => `${fileId}:${slot}`;

export function normHex(h: string | null | undefined): string | null {
  const v = (h ?? "").trim().replace(/^#/, "").toUpperCase();
  return /^[0-9A-F]{6}/.test(v) ? v.slice(0, 6) : null;
}

/** Automatic slot for a filament: material + colour ("PLA #1F8A8A"). */
export function autoKey(s: FilamentSlot): string {
  const mat = (s.material ?? "ANY").trim().toUpperCase() || "ANY";
  const hex = normHex(s.colour);
  return hex ? `${mat} #${hex}` : mat;
}

/** A filament's own slot name ("BENCHY T1", "SIGN COLOUR 2"). */
export function ownKey(file: string, s: FilamentSlot): string {
  const base = file.replace(/\.(gcode|bgcode|3mf)$/i, "").replace(/\.gcode$/i, "").replace(/[_-]+/g, " ").trim();
  return `${base} ${s.label}`.toUpperCase();
}

/** Group every filament of the picked files, in first-seen order. */
export function groupSlots(files: PickedFile[], assign: Assign): Group[] {
  const out: Group[] = [];
  for (const f of files) {
    for (const s of f.slots) {
      const key = assign[memberId(f.file_id, s.index)] ?? autoKey(s);
      let g = out.find((x) => x.key === key);
      if (!g) {
        g = { key, material: s.material, hex: normHex(s.colour), members: [], perSet: 0 };
        out.push(g);
      }
      g.members.push({ file_id: f.file_id, file: f.name, slot: s, copies: f.copies });
      g.perSet += (s.grams ?? 0) * f.copies;
    }
  }
  return out;
}

/** Materials sliced into a group (for "PLA only, as sliced"). */
export function groupMaterials(g: Group): string[] {
  return [...new Set(g.members.map((m) => (m.slot.material ?? "").toUpperCase()).filter(Boolean))];
}

/** Slot choices for one filament: existing slots, plus its own slot when not already alone. */
export function choicesFor(groups: Group[], f: PickedFile, s: FilamentSlot): { value: string; label: string; own: boolean }[] {
  const own = ownKey(f.name, s);
  const opts = groups.map((g) => ({ value: g.key, label: g.key, own: false }));
  if (!opts.some((o) => o.value === own)) opts.push({ value: own, label: `Own slot (${own})`, own: true });
  return opts;
}

/** A Library entry's filament slots (falls back to its tools for old rows). */
export function entrySlots(e: LibraryEntry): FilamentSlot[] {
  if (e.slots.length) return e.slots;
  return e.tools
    .filter((t) => t.grams == null || t.grams > 0)
    .map((t) => ({
      index: t.index,
      tool: t.index,
      label: `T${t.index}`,
      material: t.material,
      colour: t.colour,
      grams: t.grams,
      mm: t.mm,
      preset: t.preset,
      spool_id: t.spool_id,
      tray: null,
    }));
}

