/** Attention items: chip tone/symbol/word, filter groups (§3, doc 06). */
import { ATTENTION_PREFIX } from "../api/maintenance";
import type { Tone } from "../ui";

export function attentionTone(kind: string): Tone {
  if (kind === "error") return "bad";
  if (kind === "offline") return "off";
  return "warn";
}

export function attentionSymbol(kind: string): string {
  if (kind === "error") return "✖";
  if (kind === "print_finished") return "◐";
  if (kind === "offline") return "○";
  if (kind === "paused") return "‖";
  if (kind.startsWith(ATTENTION_PREFIX)) return "⚙";
  return "!";
}

export function attentionWord(kind: string): string {
  if (kind === "offline") return "Offline";
  if (kind === "error") return "Error";
  if (kind === "print_finished") return "Clear plate";
  if (kind === "paused") return "Paused";
  if (kind.startsWith(ATTENTION_PREFIX)) return "Service";
  return kind;
}

/** Filter groups on the Attention page: the chip word, so the filter reads like the list. */
export function attentionGroup(kind: string): string {
  return attentionWord(kind);
}

export function isService(kind: string): boolean {
  return kind.startsWith(ATTENTION_PREFIX);
}

export function serviceTask(kind: string): string {
  return kind.slice(ATTENTION_PREFIX.length);
}

/** Groups present in a list, in first-seen order, with counts. */
export function groupsOf(kinds: string[]): { group: string; n: number }[] {
  const m = new Map<string, number>();
  for (const k of kinds) {
    const g = attentionGroup(k);
    m.set(g, (m.get(g) ?? 0) + 1);
  }
  return [...m.entries()].map(([group, n]) => ({ group, n }));
}
