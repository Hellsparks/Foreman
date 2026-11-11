import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { DragEvent, KeyboardEvent, MouseEvent, ReactNode } from "react";
import { useOutletContext } from "react-router-dom";
import type { ShellContext } from "../shell/Shell";
import { useEventStream } from "../hooks/useEventStream";
import { useSignedUrl } from "../hooks/useSignedUrl";
import { signedUrl } from "../api/conn";
import {
  lib,
  send,
  storage,
  transfers,
  type CompatResult,
  type ConflictInfo,
  type LibraryEntry,
  type LibraryFileDetails,
  type LocRef,
  type OnConflict,
  type PrinterLocation,
  type StorageEntry,
  type TransferInfo,
} from "../api/library";
import "./Library.css";

/** Library page — one browser for the Library and each Klipper printer's G-code storage. */

type Where =
  | { kind: "library"; path: string }
  | { kind: "printer"; id: number; path: string }
  | { kind: "trash" };

type ClipMode = "copy" | "cut";

interface Item {
  key: string;
  /** As a transfer source. */
  ref: LocRef;
  /** The folder it is in. */
  where: Where;
  name: string;
  path: string;
  isDir: boolean;
  size: number;
  kind: string;
  thumb: string | null;
  time: number | null;
  grams: number | null;
  material: string | null;
  colours: string[];
  changed: Date | null;
  libId?: number;
  starred?: boolean;
  printing?: boolean;
  remote?: StorageEntry["meta"];
}

interface ConflictState {
  list: Item[];
  mode: ClipMode;
  target: Where;
  conflicts: ConflictInfo[];
  idx: number;
  applyAll: boolean;
  choices: Record<number, OnConflict>;
}

const TRANSFER_TOPICS = ["transfer.started", "transfer.progress", "transfer.finished"];
const ROOT: Where = { kind: "library", path: "" };

// ── helpers ────────────────────────────────────────────────────────────────

const whereKey = (w: Where) =>
  w.kind === "library" ? `lib:${w.path}` : w.kind === "printer" ? `p:${w.id}:${w.path}` : "trash";

const sameLoc = (a: Where, b: Where) =>
  (a.kind === "library" && b.kind === "library") ||
  (a.kind === "printer" && b.kind === "printer" && a.id === b.id);

const destRef = (w: Where): LocRef | null =>
  w.kind === "library"
    ? { kind: "library", path: w.path }
    : w.kind === "printer"
      ? { kind: "printer", printer_id: w.id, path: w.path }
      : null;

/** The folder an item opens to (folders only). */
const folderOf = (it: Item): Where | null =>
  !it.isDir || it.where.kind === "trash"
    ? null
    : it.where.kind === "printer"
      ? { kind: "printer", id: it.where.id, path: it.path }
      : { kind: "library", path: it.path };

const join = (a: string, b: string) => (a ? `${a}/${b}` : b);
const parentOf = (p: string) => (p.includes("/") ? p.slice(0, p.lastIndexOf("/")) : "");
const leafOf = (p: string) => p.slice(p.lastIndexOf("/") + 1);
const errText = (e: unknown) => (e instanceof Error ? e.message : String(e));

function kindOf(name: string, format?: string | null): string {
  if (format && format !== "other") return format.toUpperCase();
  const lower = name.toLowerCase();
  if (lower.endsWith(".gcode.3mf")) return "3MF";
  const dot = lower.lastIndexOf(".");
  return dot > 0 ? lower.slice(dot + 1).toUpperCase() : "FILE";
}

function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}

function fmtTime(secs: number | null): string {
  if (secs == null || secs <= 0) return "—";
  const m = Math.round(secs / 60);
  if (m < 60) return `${m} m`;
  return `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")} m`;
}

const fmtGrams = (g: number | null) => (g == null ? "—" : `${Math.round(g)} g`);

/** Card badge: `1h 51m` / `42m`. */
function fmtBadge(secs: number): string {
  const m = Math.round(secs / 60);
  return m < 60 ? `${m}m` : `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, "0")}m`;
}

function fmtWhen(d: Date | null): string {
  if (!d || Number.isNaN(d.getTime())) return "—";
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return "today";
  return d.toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    year: d.getFullYear() === now.getFullYear() ? undefined : "numeric",
  });
}

function fromLib(e: LibraryEntry, where: Where): Item {
  const isDir = e.kind === "folder";
  const tools = e.tools ?? [];
  const grams = tools.reduce<number | null>((a, t) => (t.grams == null ? a : (a ?? 0) + t.grams), null);
  const mats = [...new Set(tools.map((t) => t.material).filter((m): m is string => !!m))];
  return {
    key: `lib:${e.path}`,
    ref: { kind: "library", path: e.path },
    where,
    name: e.name,
    path: e.path,
    isDir,
    size: e.size,
    kind: isDir ? "folder" : kindOf(e.name, e.format),
    thumb: e.thumb_url,
    time: e.print_time_secs,
    grams,
    material: mats.length ? mats.join(" + ") : null,
    colours: tools.map((t) => t.colour).filter((c): c is string => !!c),
    changed: new Date(e.deleted_at ?? e.added_at),
    libId: e.id,
    starred: e.starred,
  };
}

function fromPrinter(e: StorageEntry, id: number, where: Where, active: string | null): Item {
  const m = e.meta;
  return {
    key: `p:${id}:${e.path}`,
    ref: { kind: "printer", printer_id: id, path: e.path },
    where,
    name: e.name,
    path: e.path,
    isDir: e.is_dir,
    size: e.size,
    kind: e.is_dir ? "folder" : kindOf(e.name),
    thumb: m?.thumb_path ? storage.thumbPath(id, m.thumb_path) : null,
    time: m?.estimated_time ?? null,
    grams: m?.filament_weight_total ?? null,
    material: m?.filament_type ?? null,
    colours: m?.filament_colors ?? [],
    changed: e.modified ? new Date(e.modified * 1000) : null,
    printing: !!active && (active === e.path || active.startsWith(`${e.path}/`)),
    remote: m,
  };
}

function Thumb({ path, alt, className, fallback }: { path: string | null; alt: string; className?: string; fallback: string }) {
  const url = useSignedUrl(path);
  const [broken, setBroken] = useState(false);
  if (!url || broken) return <span className={`fw-document-face__blank ${className ?? ""}`}>{fallback}</span>;
  return <img src={url} alt={alt} className={className} loading="lazy" onError={() => setBroken(true)} />;
}

function Chevron({ d }: { d: string }) {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="square" aria-hidden="true">
      <path d={d} />
    </svg>
  );
}

// ── page ───────────────────────────────────────────────────────────────────

export default function Library() {
  const { statuses } = useOutletContext<ShellContext>();
  const [trail, setTrail] = useState<Where[]>([ROOT]);
  const [cursor, setCursor] = useState(0);
  const where = trail[cursor];

  const [items, setItems] = useState<Item[]>([]);
  const [loadErr, setLoadErr] = useState("");
  const [rootFolders, setRootFolders] = useState<LibraryEntry[]>([]);
  const [printers, setPrinters] = useState<PrinterLocation[]>([]);
  const [trashDays, setTrashDays] = useState(30);
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState<Map<string, Item>>(new Map());
  const [selKey, setSelKey] = useState<string | null>(null);
  const [details, setDetails] = useState<LibraryFileDetails | null>(null);
  const [clip, setClip] = useState<{ items: Item[]; mode: ClipMode } | null>(null);
  const [conflict, setConflict] = useState<ConflictState | null>(null);
  const [creating, setCreating] = useState(false);
  const [fname, setFname] = useState("");
  const [ferr, setFerr] = useState("");
  const [rname, setRname] = useState("");
  const [xfers, setXfers] = useState<TransferInfo[]>([]);
  const [notice, setNotice] = useState("");
  const [undoIds, setUndoIds] = useState<number[]>([]);
  const [bulkArm, setBulkArm] = useState(false);
  const [delArm, setDelArm] = useState(false);
  const [drag, setDrag] = useState<Item[] | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  const printerName = useCallback(
    (id: number) => printers.find((p) => p.printer_id === id)?.name ?? `Printer ${id}`,
    [printers],
  );

  // ── loading ──────────────────────────────────────────────────────────────

  const load = useCallback(async () => {
    setLoadErr("");
    try {
      if (where.kind === "library") {
        const r = await lib.list(where.path);
        setItems(r.entries.map((e) => fromLib(e, where)));
      } else if (where.kind === "printer") {
        const r = await storage.list(where.id, where.path);
        setItems(r.entries.map((e) => fromPrinter(e, where.id, where, r.active_file)));
      } else {
        const r = await lib.trash();
        setTrashDays(r.purge_after_days);
        setItems(r.entries.map((e) => fromLib(e, where)));
      }
    } catch (e) {
      setItems([]);
      setLoadErr(errText(e));
    }
  }, [where]);

  const loadRoot = useCallback(async () => {
    try {
      const r = await lib.list("");
      setRootFolders(r.entries.filter((e) => e.kind === "folder"));
    } catch {
      /* sidebar keeps its last list */
    }
  }, []);

  const refresh = useCallback(() => {
    void load();
    void loadRoot();
  }, [load, loadRoot]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    void loadRoot();
    storage.locations().then((r) => setPrinters(r.printers)).catch(() => {});
    transfers.list().then((r) => setXfers(r.transfers)).catch(() => {});
  }, [loadRoot]);

  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  const onEvent = useCallback((ev: { type: string; [k: string]: unknown }) => {
    if (!ev.type.startsWith("transfer_")) return;
    const t = ev.transfer as TransferInfo;
    setXfers((prev) => {
      const i = prev.findIndex((x) => x.id === t.id);
      if (i < 0) return [...prev, t];
      const next = prev.slice();
      next[i] = t;
      return next;
    });
    if (ev.type === "transfer_finished") refreshRef.current();
  }, []);
  useEventStream(onEvent, TRANSFER_TOPICS);

  // ── selection + inspector ────────────────────────────────────────────────

  const q = query.trim().toLowerCase();
  const vis = useMemo(() => items.filter((i) => i.name.toLowerCase().includes(q)), [items, q]);
  const sel = items.find((i) => i.key === selKey) ?? null;
  const pickedList = useMemo(() => [...picked.values()], [picked]);
  const curPick = (): Item[] => (pickedList.length ? pickedList : sel ? [sel] : []);
  const isTrash = where.kind === "trash";
  const selLibId = sel && !sel.isDir && !isTrash ? sel.libId : undefined;

  const [compat, setCompat] = useState<CompatResult[]>([]);
  const [compatErr, setCompatErr] = useState<string | null>(null);
  const [compatLoading, setCompatLoading] = useState(false);
  const [sendBusy, setSendBusy] = useState(false);
  const [plateConfirm, setPlateConfirm] = useState(false);

  useEffect(() => {
    setDetails(null);
    setCompat([]);
    setCompatErr(null);
    setCompatLoading(false);
    setDelArm(false);
    setPlateConfirm(false);
    if (sel) setRname(sel.name);
  }, [sel?.key]);

  useEffect(() => {
    if (selLibId == null) return;
    let live = true;
    lib.details(selLibId).then((d) => live && setDetails(d)).catch(() => {});
    setCompatLoading(true);
    setCompatErr(null);
    send
      .check(selLibId)
      .then((r) => {
        if (live) {
          setCompat(r);
          setCompatLoading(false);
        }
      })
      .catch((e) => {
        if (live) {
          setCompat([]);
          setCompatErr(errText(e));
          setCompatLoading(false);
        }
      });
    return () => {
      live = false;
    };
  }, [selLibId]);

  const selIsPrintable = sel && !sel.isDir && sel.kind !== "STL" && sel.kind !== "folder";

  const doPrintNow = async (printerId: number) => {
    if (!sel?.libId || sendBusy) return;
    setSendBusy(true);
    try {
      const r = await send.send({
        file_id: sel.libId,
        printer_id: printerId,
        mode: "print_now",
        plate_confirm: plateConfirm,
      });
      setNotice(r.message);
    } catch (e) {
      setNotice(errText(e));
    } finally {
      setSendBusy(false);
    }
  };

  // ── navigation ───────────────────────────────────────────────────────────

  const go = (w: Where) => {
    if (whereKey(w) === whereKey(where)) return;
    setTrail([...trail.slice(0, cursor + 1), w]);
    setCursor(cursor + 1);
    setQuery("");
    setSelKey(null);
    setCreating(false);
    setBulkArm(false);
  };
  const upTarget: Where | null =
    where.kind === "library" && where.path
      ? { kind: "library", path: parentOf(where.path) }
      : where.kind === "printer" && where.path
        ? { kind: "printer", id: where.id, path: parentOf(where.path) }
        : null;
  const step = (d: number) => {
    setCursor(cursor + d);
    setQuery("");
    setSelKey(null);
  };

  const crumbs: { label: string; w: Where }[] = (() => {
    if (where.kind === "trash") return [{ label: "Trash", w: where }];
    const out: { label: string; w: Where }[] =
      where.kind === "library"
        ? [{ label: "All files", w: ROOT }]
        : [{ label: printerName(where.id), w: { kind: "printer", id: where.id, path: "" } }];
    let acc = "";
    for (const seg of where.path.split("/").filter(Boolean)) {
      acc = join(acc, seg);
      out.push({
        label: seg,
        w: where.kind === "library" ? { kind: "library", path: acc } : { kind: "printer", id: where.id, path: acc },
      });
    }
    return out;
  })();
  const locTitle = `${crumbs[crumbs.length - 1].label} contents`;
  const topNote =
    where.kind === "library"
      ? "FOREMAN LIBRARY · ON THE SERVER"
      : where.kind === "printer"
        ? `${printerName(where.id).toUpperCase()} · MOONRAKER · GCODES`
        : `TRASH · KEPT ${trashDays} DAYS`;
  const whereLabel = (w: Where) =>
    w.kind === "library"
      ? w.path ? leafOf(w.path) : "All files"
      : w.kind === "printer"
        ? w.path ? leafOf(w.path) : printerName(w.id)
        : "Trash";

  // ── transfers ────────────────────────────────────────────────────────────

  const startPaste = async (list: Item[], mode: ClipMode, target: Where, per: Record<number, OnConflict> = {}) => {
    const dest = destRef(target);
    if (!dest || !list.length) return;
    try {
      const out = await transfers.start({
        items: list.map((it, i) => ({ src: it.ref, on_conflict: per[i] })),
        dest,
        mode: mode === "cut" ? "move" : "copy",
        on_conflict: "ask",
      });
      if (out.outcome === "conflicts") {
        setConflict({ list, mode, target, conflicts: out.conflicts, idx: 0, applyAll: out.conflicts.length > 1, choices: per });
        return;
      }
      setConflict(null);
      setXfers((prev) => [...prev.filter((x) => !out.transfers.some((t) => t.id === x.id)), ...out.transfers]);
      if (mode === "cut") setClip(null);
      setPicked(new Map());
      setUndoIds([]);
      const n = out.transfers.length;
      setNotice(
        (n
          ? `${n} item(s) ${mode === "cut" ? "moving" : "copying"} to ${whereLabel(target)}`
          : `Nothing ${mode === "cut" ? "moved" : "copied"}`) +
          (out.skipped.length ? ` · ${out.skipped.length} skipped` : "") +
          (out.blocked.length ? ` · blocked: ${out.blocked.map((b) => `${b.name} (${b.reason})`).join(", ")}` : ""),
      );
    } catch (e) {
      setNotice(errText(e));
    }
  };

  const resolve = (choice: OnConflict | null) => {
    const c = conflict;
    if (!c) return;
    if (!choice) {
      setConflict(null);
      return;
    }
    const choices = { ...c.choices };
    const upto = c.applyAll ? c.conflicts.length : c.idx + 1;
    for (let i = c.idx; i < upto; i++) choices[c.conflicts[i].index] = choice;
    if (upto < c.conflicts.length) {
      setConflict({ ...c, idx: upto, choices });
      return;
    }
    void startPaste(c.list, c.mode, c.target, choices);
  };

  const clipItems = (list: Item[], mode: ClipMode) => {
    if (!list.length || isTrash) return;
    setClip({ items: list, mode });
    setNotice(`${mode === "copy" ? "Copied" : "Cut"} ${list.length} item(s). Open a folder or printer and Paste.`);
  };

  const paste = () => {
    if (clip) void startPaste(clip.items, clip.mode, where);
  };

  const cancelXfer = async (id: string) => {
    try {
      const t = await transfers.cancel(id);
      setXfers((prev) => prev.map((x) => (x.id === t.id ? t : x)));
    } catch (e) {
      setNotice(errText(e));
    }
  };

  // ── delete / restore ─────────────────────────────────────────────────────

  const doDelete = async (list: Item[]) => {
    const parts: string[] = [];
    const libItems = list.filter((i) => i.where.kind === "library" && i.libId != null);
    let undo: number[] = [];
    try {
      if (libItems.length) {
        const r = await lib.trashMany(libItems.map((i) => i.libId as number));
        undo = r.trashed;
        const done = libItems.filter((i) => r.trashed.includes(i.libId as number));
        if (done.length) parts.push(`Moved to trash: ${done.map((i) => i.name + (i.isDir ? "/" : "")).join(", ")}.`);
        if (r.errors.length) parts.push(`Not moved: ${r.errors.map((e) => e.error).join("; ")}.`);
      }
      const byPrinter = new Map<number, Item[]>();
      for (const i of list) {
        if (i.where.kind === "printer") byPrinter.set(i.where.id, [...(byPrinter.get(i.where.id) ?? []), i]);
      }
      for (const [id, its] of byPrinter) {
        const r = await storage.remove(id, its.map((i) => i.path));
        if (r.deleted.length) {
          parts.push(`Deleted from ${printerName(id)}: ${r.deleted.map(leafOf).join(", ")}. Printer files can't be restored.`);
        }
        if (r.blocked.length) parts.push(`Not deleted: ${r.blocked.map((b) => `${b.name} (${b.reason})`).join(", ")}.`);
      }
    } catch (e) {
      parts.push(errText(e));
    }
    setUndoIds(undo);
    setNotice(parts.join(" ") || "Nothing deleted.");
    setPicked(new Map());
    setSelKey(null);
    setBulkArm(false);
    setDelArm(false);
    refresh();
  };

  const restore = async (ids: number[], msg?: string) => {
    if (!ids.length) return;
    try {
      const r = await lib.restoreMany(ids);
      setNotice(msg ?? `Restored ${r.restored.map((e) => e.name).join(", ")}.`);
    } catch (e) {
      setNotice(errText(e));
    }
    setUndoIds([]);
    setPicked(new Map());
    setSelKey(null);
    refresh();
  };

  const bulkDel = () => {
    const list = curPick();
    if (!list.length) return;
    if (isTrash) {
      void restore(list.map((i) => i.libId as number));
      return;
    }
    if (!bulkArm) {
      setBulkArm(true);
      return;
    }
    void doDelete(list);
  };

  // ── create / upload / rename / star / download ───────────────────────────

  const createFolder = async () => {
    const v = fname.trim();
    const err = !v
      ? "Enter a name."
      : /[\\/]/.test(v)
        ? "Use a name without slashes."
        : items.some((i) => i.name.toLowerCase() === v.toLowerCase())
          ? "That name already exists in this folder."
          : "";
    if (err) {
      setFerr(err);
      return;
    }
    try {
      if (where.kind === "library") await lib.mkdir(where.path, v);
      else if (where.kind === "printer") await storage.mkdir(where.id, where.path, v);
      setCreating(false);
      setFname("");
      setNotice(`Created ${v}.`);
      refresh();
    } catch (e) {
      setFerr(errText(e));
    }
  };

  const onUpload = async (files: FileList | null) => {
    if (!files?.length) return;
    const list = [...files];
    setNotice(`Uploading ${list.length} file(s)…`);
    try {
      if (where.kind === "library") await lib.upload(where.path, list);
      else if (where.kind === "printer") await storage.upload(where.id, where.path, list);
      setNotice(`Uploaded ${list.map((f) => f.name).join(", ")} to ${whereLabel(where)}.`);
      refresh();
    } catch (e) {
      setNotice(errText(e));
    }
    if (fileInput.current) fileInput.current.value = "";
  };

  const rename = async () => {
    if (!sel) return;
    const v = rname.trim();
    if (!v || v === sel.name) return;
    if (/[\\/]/.test(v)) {
      setNotice("Use a name without slashes.");
      return;
    }
    try {
      if (sel.where.kind === "library" && sel.libId != null) {
        const e = await lib.patch(sel.libId, { name: v });
        setSelKey(`lib:${e.path}`);
      } else if (sel.where.kind === "printer") {
        const r = await storage.rename(sel.where.id, sel.path, v);
        setSelKey(`p:${sel.where.id}:${r.path}`);
      }
      setNotice(`Renamed to ${v}.`);
      refresh();
    } catch (e) {
      setNotice(errText(e));
    }
  };

  const toggleStar = async () => {
    if (!sel || sel.libId == null) return;
    try {
      await lib.patch(sel.libId, { starred: !sel.starred });
      void load();
    } catch (e) {
      setNotice(errText(e));
    }
  };

  const download = async (it: Item) => {
    const path =
      it.where.kind === "printer" ? storage.downloadPath(it.where.id, it.path) : lib.downloadPath(it.libId as number);
    const a = document.createElement("a");
    a.href = await signedUrl(path);
    a.download = it.name;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setNotice(`Downloading ${it.name}.`);
  };

  // ── keyboard + drag ──────────────────────────────────────────────────────

  const selectAll = () => setPicked(new Map(vis.map((i) => [i.key, i])));
  const togglePick = (it: Item) =>
    setPicked((prev) => {
      const next = new Map(prev);
      if (next.has(it.key)) next.delete(it.key);
      else next.set(it.key, it);
      return next;
    });

  const onKeys = (e: KeyboardEvent<HTMLDivElement>) => {
    const tag = (e.target as HTMLElement).tagName;
    if (tag === "INPUT" || tag === "TEXTAREA") return;
    if (e.key === "Delete" || e.key === "Backspace") {
      if (curPick().length) {
        e.preventDefault();
        bulkDel();
      }
      return;
    }
    if (e.key === "Escape") {
      setBulkArm(false);
      setPicked(new Map());
      return;
    }
    if (!(e.ctrlKey || e.metaKey)) return;
    const k = e.key.toLowerCase();
    if (k === "c") clipItems(curPick(), "copy");
    else if (k === "x") clipItems(curPick(), "cut");
    else if (k === "v") paste();
    else if (k === "a") selectAll();
    else return;
    e.preventDefault();
  };

  const dropOn = (target: Where) => ({
    onDragOver: (e: DragEvent) => {
      if (drag && target.kind !== "trash") e.preventDefault();
    },
    onDrop: (e: DragEvent) => {
      e.preventDefault();
      const d = drag;
      setDrag(null);
      if (!d?.length) return;
      void startPaste(d, sameLoc(d[0].where, target) ? "cut" : "copy", target);
    },
    "data-drop": drag && target.kind !== "trash" ? "true" : undefined,
  });

  // ── derived UI ───────────────────────────────────────────────────────────

  const pickN = pickedList.length;
  const hereKey = whereKey(where);
  const pickElsewhere = pickN > 0 && pickedList.some((i) => whereKey(i.where) !== hereKey);
  const pickHere = pickN > 0 && !pickElsewhere;
  const hereOff = isTrash || pickedList.some((i) => {
    const f = folderOf(i);
    return !!f && whereKey(f) === hereKey;
  });
  const pickText =
    `${pickN} selected` +
    (pickElsewhere
      ? ` in ${[...new Set(pickedList.map((i) => whereLabel(i.where)))].join(", ")} · open the target folder or printer, then Copy here or Move here`
      : " · tick more, or open another folder to copy / move them there");
  const curN = curPick().length;
  const bulkDelLabel = isTrash ? "Restore" : bulkArm ? `Delete ${curN}? Click again` : "Delete";
  const clipFrom = clip?.items[0]?.where;
  const pasteLabel = clip
    ? `Paste ${clip.items.length}${clipFrom && !sameLoc(clipFrom, where) ? ` · from ${clipFrom.kind === "printer" ? printerName(clipFrom.id) : "Library"}` : ""}`
    : "Paste";
  const running = xfers.filter((x) => x.state === "queued" || x.state === "running").length;
  const c = conflict;
  const cur = c?.conflicts[c.idx];
  const conflictText = !c
    ? ""
    : c.applyAll
      ? `${c.conflicts.length - c.idx} already here: ${c.conflicts.slice(c.idx).map((x) => x.name).join(", ")}. What should happen?`
      : `${cur?.name} is already here (${c.idx + 1} of ${c.conflicts.length})${cur?.dest_printing ? " and printing now, so it can't be replaced" : ""}. What should happen?`;
  const replaceOff = !c ? true : c.applyAll ? c.conflicts.slice(c.idx).every((x) => x.dest_printing) : !!cur?.dest_printing;

  const libLocs: { name: string; icon: string; w: Where }[] = [
    { name: "All files", icon: "⊞", w: ROOT },
    ...rootFolders.map((f) => ({ name: f.name, icon: "▱", w: { kind: "library", path: f.path } as Where })),
  ];
  const isCurLoc = (w: Where) =>
    w.kind === "printer" ? where.kind === "printer" && where.id === w.id : whereKey(w) === hereKey;

  const printerDot = (p: PrinterLocation) => {
    const s = statuses[p.printer_id]?.state ?? p.state;
    if (!p.browsable || s === "offline") return "var(--fw-muted)";
    if (s === "printing" || s === "paused") return "var(--fw-info)";
    if (s === "error") return "var(--fw-danger)";
    return "var(--fw-success)";
  };

  const facts: { k: string; v: ReactNode }[] = (() => {
    if (!sel) return [];
    if (sel.isDir) return [{ k: "CHANGED", v: fmtWhen(sel.changed) }];
    const meta = details?.meta;
    const tools = meta?.tools ?? [];
    const colours = tools.length ? tools.map((t) => t.colour).filter((x): x is string => !!x) : sel.colours;
    const swatches = colours.length ? (
      <span className="fw-swatches" aria-label={`Colours ${colours.join(", ")}`}>
        {colours.map((col, i) => (
          <i key={i} style={{ background: col }} title={col} />
        ))}
      </span>
    ) : (
      "—"
    );
    const slicer = meta?.slicer ?? sel.remote?.slicer ?? null;
    const slicerV = meta?.slicer_version ?? sel.remote?.slicer_version ?? null;
    const nozzle = meta?.nozzle_diameter ?? sel.remote?.nozzle_diameter ?? null;
    const preset = tools.find((t) => t.preset)?.preset ?? sel.remote?.filament_name ?? null;
    return [
      { k: "FILAMENT", v: sel.material ?? (sel.kind === "STL" ? "not sliced" : "—") },
      { k: "TIME", v: fmtTime(sel.time) },
      { k: "GRAMS", v: fmtGrams(sel.grams) },
      { k: "COLOURS", v: swatches },
      { k: "SLICER", v: slicer ? `${slicer}${slicerV ? ` ${slicerV}` : ""}` : "—" },
      { k: "PROFILE", v: meta?.printer_settings_id ?? "—" },
      { k: "FILAMENT PRESET", v: preset ?? "—" },
      { k: "NOZZLE", v: nozzle ? `${nozzle} mm` : "—" },
      { k: "CHANGED", v: fmtWhen(sel.changed) },
    ];
  })();

  const footCount =
    `${vis.length} OF ${items.length} ITEMS` +
    (pickN ? ` · ${pickN} SELECTED` : "") +
    (clip ? ` · CLIPBOARD: ${clip.items.length} (${clip.mode})` : "");

  const pickedHere = (mode: ClipMode) =>
    void startPaste(pickedList.filter((i) => whereKey(i.where) !== hereKey), mode, where);

  // ── render ───────────────────────────────────────────────────────────────

  return (
    <div className="lib-page">
      <div className="fw-browser" data-view="flat" tabIndex={-1} onKeyDown={onKeys}>
        <div className="fw-browser-tools">
          <div className="fw-browser-actions">
            <button type="button" className="fb-button" aria-label="Back" disabled={cursor === 0} onClick={() => step(-1)}>
              <Chevron d="M15 5l-7 7 7 7" />
            </button>
            <button type="button" className="fb-button" aria-label="Forward" disabled={cursor >= trail.length - 1} onClick={() => step(1)}>
              <Chevron d="M9 5l7 7-7 7" />
            </button>
            <button type="button" className="fb-button" aria-label="Up one folder" disabled={!upTarget} onClick={() => upTarget && go(upTarget)}>
              <Chevron d="M5 15l7-7 7 7" />
            </button>
          </div>
          <label className="fw-browser-search">
            <span className="fw-sr">Search this folder</span>
            <input type="search" placeholder="Search this folder…" value={query} onChange={(e) => setQuery(e.target.value)} />
          </label>
          <div className="fw-browser-actions" role="group" aria-label="File actions">
            <button type="button" className="fb-button" onClick={() => clipItems(curPick(), "copy")} disabled={!curN || isTrash}>
              Copy
            </button>
            <button type="button" className="fb-button" onClick={() => clipItems(curPick(), "cut")} disabled={!curN || isTrash}>
              Cut
            </button>
            <button type="button" className={`fb-button${curN && !isTrash ? " fb-danger" : ""}`} onClick={bulkDel} disabled={!curN}>
              {bulkDelLabel}
            </button>
            <button
              type="button"
              className="fb-button"
              onClick={paste}
              disabled={!clip || isTrash}
              style={clip ? { borderColor: "var(--fw-accent)" } : undefined}
            >
              {pasteLabel}
            </button>
            <button
              type="button"
              className="fb-button"
              onClick={() => {
                setCreating(!creating);
                setFname("");
                setFerr("");
              }}
              disabled={isTrash}
            >
              + Folder
            </button>
            <button type="button" className="fb-button fb-primary" onClick={() => fileInput.current?.click()} disabled={isTrash}>
              Upload
            </button>
            <input ref={fileInput} type="file" multiple hidden onChange={(e) => void onUpload(e.target.files)} />
          </div>
        </div>

        <nav className="fw-browser-crumbs" aria-label="Folder breadcrumbs">
          {crumbs.map((cr, i) => (
            <span key={whereKey(cr.w)} style={{ display: "contents" }}>
              {i > 0 && (
                <span className="fw-browser-crumbs__sep" aria-hidden="true">
                  /
                </span>
              )}
              <button type="button" onClick={() => go(cr.w)} aria-current={i === crumbs.length - 1 ? "page" : undefined}>
                {cr.label}
              </button>
            </span>
          ))}
          <span className="fw-browser-crumbs__note">{topNote}</span>
        </nav>

        {pickN > 0 && (
          <div className="fw-browser-banner">
            <span>{pickText}</span>
            {pickElsewhere && !isTrash && (
              <>
                <button type="button" className="fb-button fb-primary fb-small" disabled={hereOff} onClick={() => pickedHere("copy")}>
                  Copy {pickN} here
                </button>
                <button type="button" className="fb-button fb-small" disabled={hereOff} onClick={() => pickedHere("cut")}>
                  Move here
                </button>
              </>
            )}
            {pickHere && !isTrash && (
              <>
                <button type="button" className="fb-button fb-small" onClick={() => clipItems(pickedList, "copy")}>
                  Copy
                </button>
                <button type="button" className="fb-button fb-small" onClick={() => clipItems(pickedList, "cut")}>
                  Cut
                </button>
              </>
            )}
            <button type="button" className={`fb-button fb-small${isTrash ? "" : " fb-danger"}`} onClick={bulkDel}>
              {bulkDelLabel}
            </button>
            <button type="button" className="fb-button fb-small" onClick={selectAll}>
              Select all
            </button>
            <button
              type="button"
              className="fb-button fb-small"
              onClick={() => {
                setPicked(new Map());
                setBulkArm(false);
              }}
            >
              Clear
            </button>
          </div>
        )}

        {c && (
          <div className="fw-browser-banner fw-browser-banner--warn" role="alertdialog" aria-label="Name conflict">
            <span>{conflictText}</span>
            {c.conflicts.length - c.idx > 1 && (
              <label>
                <input type="checkbox" checked={c.applyAll} onChange={(e) => setConflict({ ...c, applyAll: e.target.checked })} />
                Apply to all
              </label>
            )}
            <button type="button" className="fb-button fb-small" onClick={() => resolve("keep_both")}>
              Keep both
            </button>
            <button type="button" className="fb-button fb-small" onClick={() => resolve("replace")} disabled={replaceOff}>
              Replace
            </button>
            <button type="button" className="fb-button fb-small" onClick={() => resolve("skip")}>
              {c.applyAll ? "Skip those" : "Skip"}
            </button>
            <button type="button" className="fb-button fb-small" onClick={() => resolve(null)}>
              Cancel
            </button>
          </div>
        )}

        {creating && (
          <div className="fw-browser-form">
            <label>
              New folder name
              <input
                type="text"
                value={fname}
                maxLength={80}
                autoFocus
                onChange={(e) => setFname(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && void createFolder()}
              />
            </label>
            <button type="button" className="fb-button" onClick={() => void createFolder()}>
              Create folder
            </button>
            <button type="button" className="fb-button" onClick={() => setCreating(false)}>
              Cancel
            </button>
            {ferr && (
              <p className="fw-browser-error" role="alert">
                {ferr}
              </p>
            )}
          </div>
        )}

        <div className="fw-browser-body" data-inspector={sel ? "true" : undefined}>
          <nav className="fw-browser-locations" aria-label="File locations">
            <span>LIBRARY</span>
            {libLocs.map((l) => (
              <button key={whereKey(l.w)} type="button" aria-current={isCurLoc(l.w) ? "page" : undefined} onClick={() => go(l.w)} {...dropOn(l.w)}>
                <span aria-hidden="true">{l.icon}</span>
                <span className="fw-loc-name">{l.name}</span>
              </button>
            ))}
            <button type="button" aria-current={isTrash ? "page" : undefined} onClick={() => go({ kind: "trash" })}>
              <span aria-hidden="true">⌫</span>
              <span className="fw-loc-name">Trash</span>
            </button>
            <span>PRINTERS</span>
            {printers.length === 0 && <span style={{ letterSpacing: 0 }}>No Klipper printers yet.</span>}
            {printers.map((p) => {
              const w: Where = { kind: "printer", id: p.printer_id, path: "" };
              return (
                <button key={p.printer_id} type="button" aria-current={isCurLoc(w) ? "page" : undefined} onClick={() => go(w)} {...dropOn(w)}>
                  <span aria-hidden="true" style={{ color: printerDot(p) }}>
                    ●
                  </span>
                  <span className="fw-loc-name">
                    {p.name}
                    <br />
                    <span className="fw-loc-sub">{p.browsable ? "Moonraker · gcodes" : "not connected"}</span>
                  </span>
                </button>
              );
            })}
          </nav>

          <div className="fw-browser-content">
            <h3 className="fw-file-location-title">{locTitle}</h3>
            {vis.length > 0 ? (
              <div className="fw-file-grid" data-picking={pickN > 0 ? "true" : undefined}>
                {vis.map((it) => {
                  const pk = picked.has(it.key);
                  const isCut = clip?.mode === "cut" && clip.items.some((x) => x.key === it.key);
                  const meta = it.isDir
                    ? "Folder"
                    : it.printing
                      ? "▶ printing now"
                      : isTrash
                        ? `deleted ${fmtWhen(it.changed)}`
                        : it.grams != null
                          ? `${it.material ?? it.kind} · ${fmtGrams(it.grams)}`
                          : `${it.kind} · ${fmtSize(it.size)}`;
                  const target = folderOf(it);
                  const drop = target && !(drag ?? []).some((d) => d.key === it.key) ? dropOn(target) : {};
                  return (
                    <div
                      key={it.key}
                      className="fw-file-cell"
                      draggable={!isTrash}
                      onDragStart={(e) => {
                        e.dataTransfer.setData("text/plain", it.name);
                        e.dataTransfer.effectAllowed = "copyMove";
                        setDrag(pk ? pickedList : [it]);
                      }}
                      onDragEnd={() => setTimeout(() => setDrag(null), 50)}
                    >
                      <button
                        type="button"
                        role="checkbox"
                        className="fw-file-tick"
                        aria-checked={pk}
                        aria-label={`Select ${it.name}`}
                        onClick={() => {
                          setBulkArm(false);
                          togglePick(it);
                        }}
                      >
                        {pk ? "✓" : ""}
                      </button>
                      <button
                        type="button"
                        className="fw-file-card"
                        aria-label={`${target ? "Open folder" : "Inspect file"} ${it.name}`}
                        aria-pressed={selKey === it.key || pk}
                        data-cut={isCut ? "true" : undefined}
                        {...drop}
                        onClick={(e: MouseEvent) => {
                          if (e.ctrlKey || e.metaKey || e.shiftKey) {
                            togglePick(it);
                            return;
                          }
                          if (target) go(target);
                          else setSelKey(it.key);
                        }}
                      >
                        <span className="fw-file-art" aria-hidden="true">
                          <span className="fw-file-object">
                            {it.isDir ? (
                              <>
                                <span className="fw-folder-back" />
                                <span className="fw-folder-paper" />
                                <span className="fw-folder-front" />
                              </>
                            ) : (
                              <span className="fw-document-face">
                                <Thumb path={it.thumb} alt="" fallback={it.kind} />
                                <b>
                                  {it.kind}
                                  <span>{it.time ? fmtBadge(it.time) : fmtSize(it.size)}</span>
                                </b>
                              </span>
                            )}
                          </span>
                        </span>
                        <strong>
                          {it.name}
                          {it.starred ? " ★" : ""}
                        </strong>
                        <small data-printing={it.printing ? "true" : undefined}>{meta}</small>
                      </button>
                    </div>
                  );
                })}
              </div>
            ) : (
              <div className="fw-browser-empty">
                <strong>
                  {loadErr ? `Can't open ${whereLabel(where)}.` : q ? "No matching files." : isTrash ? "The trash is empty." : "An open space."}
                </strong>
                <span>
                  {loadErr ||
                    (q
                      ? "Try another name or clear your search."
                      : isTrash
                        ? `Deleted Library files stay here for ${trashDays} days.`
                        : "Upload a file or create a folder.")}
                </span>
                {loadErr ? (
                  <button type="button" className="fb-button" onClick={() => void load()}>
                    Try again
                  </button>
                ) : q ? (
                  <button type="button" className="fb-button" onClick={() => setQuery("")}>
                    Clear search
                  </button>
                ) : !isTrash ? (
                  <button type="button" className="fb-button" onClick={() => fileInput.current?.click()}>
                    Upload
                  </button>
                ) : null}
              </div>
            )}
          </div>

          {sel && (
            <aside className="fw-file-inspector" aria-label="File inspector">
              {!sel.isDir && (
                <div>
                  <Thumb path={sel.thumb} alt={`Preview of ${sel.name}`} className="fw-inspector-preview" fallback={sel.kind} />
                </div>
              )}
              <div className="fw-inspector-head">
                <div>
                  <span className="fw-inspector-eyebrow">
                    FILE INSPECTOR / {sel.isDir ? "FOLDER" : sel.kind} / {sel.isDir ? "—" : fmtSize(sel.size)} /{" "}
                    {sel.where.kind === "printer" ? printerName(sel.where.id).toUpperCase() : isTrash ? "TRASH" : "LIBRARY"}
                  </span>
                  <h3>
                    {sel.name}
                    {sel.starred ? " ★" : ""}
                  </h3>
                </div>
                <button type="button" className="fb-button" onClick={() => setSelKey(null)}>
                  Close ×
                </button>
              </div>
              <div className="fw-facts">
                {facts.map((f) => (
                  <div key={f.k} className="fw-fact">
                    <div>{f.k}</div>
                    <div>{f.v}</div>
                  </div>
                ))}
              </div>
              {sel.printing && (
                <p className="fw-inspector-eyebrow" style={{ color: "var(--fw-info)" }}>
                  ▶ PRINTING NOW · CAN'T BE MOVED, REPLACED OR DELETED
                </p>
              )}
              <div className="fw-browser-actions">
                {isTrash ? (
                  <button type="button" className="fb-button fb-primary" onClick={() => void restore([sel.libId as number])}>
                    Restore
                  </button>
                ) : (
                  <>
                    {!sel.isDir && sel.kind === "STL" && (
                      <button type="button" className="fb-button" disabled title="STL files must be sliced first">
                        Slice first to print
                      </button>
                    )}
                    <button type="button" className="fb-button" onClick={() => clipItems([sel], "copy")}>
                      Copy
                    </button>
                    <button type="button" className="fb-button" onClick={() => clipItems([sel], "cut")} disabled={sel.printing}>
                      Cut
                    </button>
                    {sel.libId != null && (
                      <button type="button" className="fb-button" aria-pressed={!!sel.starred} onClick={() => void toggleStar()}>
                        {sel.starred ? "★ Starred" : "☆ Star"}
                      </button>
                    )}
                    {!sel.isDir && (
                      <button type="button" className="fb-button" onClick={() => void download(sel)}>
                        Download
                      </button>
                    )}
                    <button
                      type="button"
                      className="fb-button fb-danger"
                      disabled={sel.printing}
                      onClick={() => (delArm ? void doDelete([sel]) : setDelArm(true))}
                    >
                      {sel.printing
                        ? "Printing now"
                        : delArm
                          ? sel.where.kind === "library"
                            ? "Move to trash? Click again"
                            : "Delete from printer? Click again"
                          : sel.where.kind === "library"
                            ? "Move to trash"
                            : "Delete"}
                    </button>
                  </>
                )}
              </div>
              {selIsPrintable && (
                <div className="fw-send-panel">
                  <span className="fw-inspector-eyebrow">SEND TO PRINTER</span>
                  {compatLoading && <span className="fw-send-loading">Checking printers…</span>}
                  {compatErr && <span className="fw-send-err">{compatErr}</span>}
                  {!compatLoading && !compatErr && compat.length === 0 && (
                    <span className="fw-send-empty">No printers configured</span>
                  )}
                  <div className="fw-send-list">
                    {compat.map((c) => {
                      const verdictColor =
                        c.verdict === "ready"
                          ? "var(--fw-success)"
                          : c.verdict === "plate"
                            ? "var(--fw-warning)"
                            : c.verdict === "busy" || c.verdict === "offline"
                              ? "var(--fw-muted)"
                              : "var(--fw-danger)";
                      return (
                        <div key={Number(c.printer_id)} className="fw-send-row">
                          <span style={{ color: verdictColor }}>●</span>
                          <span className="fw-send-name">{c.printer_name}</span>
                          <span className="fw-send-reason">{c.reason}</span>
                          {(c.verdict === "ready" || c.verdict === "plate") && (
                            <button
                              type="button"
                              className="fb-button fb-primary fb-small"
                              disabled={sendBusy || (c.verdict === "plate" && !plateConfirm)}
                              onClick={() => void doPrintNow(Number(c.printer_id))}
                            >
                              Print now
                            </button>
                          )}
                          {c.quick_fix && c.verdict !== "ready" && c.verdict !== "plate" && (
                            <span className="fw-send-fix">{c.quick_fix}</span>
                          )}
                        </div>
                      );
                    })}
                  </div>
                  {compat.some((c) => c.verdict === "plate") && (
                    <label className="fw-send-plate">
                      <input
                        type="checkbox"
                        checked={plateConfirm}
                        onChange={(e) => setPlateConfirm(e.target.checked)}
                      />
                      Plate is clear — I removed the last print
                    </label>
                  )}
                  <div className="fw-browser-actions">
                    <button type="button" className="fb-button" disabled title="Queue — coming next session">
                      Queue
                    </button>
                    <button type="button" className="fb-button" disabled title="Fleet queue — coming next session">
                      Fleet queue
                    </button>
                  </div>
                </div>
              )}
              {!isTrash && (
                <div className="fw-browser-form" style={{ padding: 0, border: 0 }}>
                  <label>
                    Rename {sel.isDir ? "folder" : "file"}
                    <input
                      type="text"
                      value={rname}
                      maxLength={120}
                      disabled={sel.printing}
                      onChange={(e) => setRname(e.target.value)}
                      onKeyDown={(e) => e.key === "Enter" && void rename()}
                    />
                  </label>
                  <button
                    type="button"
                    className="fb-button"
                    onClick={() => void rename()}
                    disabled={sel.printing || !rname.trim() || rname.trim() === sel.name}
                  >
                    Save name
                  </button>
                </div>
              )}
            </aside>
          )}
        </div>

        {xfers.length > 0 && (
          <div className="fw-browser-xfers" aria-label="Transfers">
            <div className="fw-browser-xfers__head">
              <span>
                TRANSFERS · {running} running · {xfers.length - running} done
              </span>
              <button
                type="button"
                className="fb-button fb-tiny"
                onClick={() => {
                  void transfers.clear().catch(() => {});
                  setXfers((prev) => prev.filter((x) => x.state === "queued" || x.state === "running"));
                }}
              >
                Clear finished
              </button>
            </div>
            {xfers.map((x) => {
              const pct =
                x.state === "done" ? 100 : x.bytes_total ? Math.min(100, Math.floor((x.bytes_done / x.bytes_total) * 100)) : 0;
              const label = x.state === "running" ? `${pct}%` : x.state === "done" && x.mode === "move" ? "moved" : x.state;
              const live = x.state === "queued" || x.state === "running";
              return (
                <div key={x.id} className="fw-xfer" data-state={x.state} title={x.error ?? x.note ?? undefined}>
                  <span>{x.name}</span>
                  <span>{x.error ? `${x.route} · ${x.error}` : x.note ? `${x.route} · ${x.note}` : x.route}</span>
                  <span className="fw-xfer__bar" aria-hidden="true">
                    <span style={{ width: `${pct}%` }} />
                  </span>
                  <span className="fw-xfer__state">{label}</span>
                  {live ? (
                    <button type="button" className="fb-button fb-tiny" onClick={() => void cancelXfer(x.id)}>
                      Cancel
                    </button>
                  ) : (
                    <span />
                  )}
                </div>
              );
            })}
          </div>
        )}

        <div className="fw-browser-foot">
          <span>{footCount}</span>
          <span role="status">
            {notice}
            {undoIds.length > 0 && (
              <button type="button" className="fb-button fb-tiny" onClick={() => void restore(undoIds, "Restored.")}>
                Undo
              </button>
            )}
          </span>
        </div>
      </div>
    </div>
  );
}
