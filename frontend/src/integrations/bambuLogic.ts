/** Pure helpers for Settings → Integrations → Bambu Cloud. */
import type { BambuAccountView, CodeChannel, DeviceRow } from "../api/bambu";
import type { Tone } from "../ui/StatusChip";

export interface StateChip {
  tone: Tone;
  symbol: string;
  word: string;
  /** Second line under the chip ("until 16 Feb 2027", the error text…). */
  detail: string | null;
}

export function fmtDate(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}

export function stateChip(a: BambuAccountView): StateChip {
  switch (a.state) {
    case "connected":
      return {
        tone: "ok",
        symbol: "●",
        word: "Connected",
        detail: a.token_expires_at ? `until ${fmtDate(a.token_expires_at)}` : "expiry not given by Bambu",
      };
    case "expiring": {
      const d = a.days_left ?? 0;
      return {
        tone: "warn",
        symbol: "◐",
        word: d === 0 ? "Expires today" : `Expires in ${d} day${d === 1 ? "" : "s"}`,
        detail: a.token_expires_at ? `on ${fmtDate(a.token_expires_at)} — re-authenticate before then` : null,
      };
    }
    case "expired":
      return {
        tone: "bad",
        symbol: "✖",
        word: "Expired",
        detail: a.token_rejected_at
          ? `Bambu refused the sign-in on ${fmtDate(a.token_rejected_at)} — re-authenticate`
          : a.token_expires_at
            ? `on ${fmtDate(a.token_expires_at)} — re-authenticate`
            : "re-authenticate",
      };
    case "error":
      return { tone: "bad", symbol: "✖", word: "Error", detail: a.last_error };
    case "code_required":
      return { tone: "info", symbol: "…", word: "Code needed", detail: codePrompt(a.code_channel) };
    case "signed_out":
      return { tone: "off", symbol: "○", word: "Signed out", detail: null };
  }
}

export function codePrompt(ch: CodeChannel | null): string {
  return ch === "authenticator"
    ? "Enter the 6-digit code from your authenticator app."
    : "Bambu emailed you a 6-digit code. Enter it here.";
}

/** "Remove" confirmation copy, naming how many printers reference the account. */
export function removeWarning(a: BambuAccountView): string {
  const n = a.printer_count;
  if (n === 0) return "No printers reference this account.";
  return `${n} printer${n === 1 ? " references" : "s reference"} this account. ${n === 1 ? "It" : "They"} will be unlinked from Bambu Cloud (LAN settings are kept).`;
}

/** Can this account's device list be fetched? */
export function canListDevices(a: BambuAccountView): boolean {
  return a.has_token && a.state !== "expired";
}

/** What a device row offers: already a Foreman printer on this account, attach to the
 *  printer with its serial, or create a printer. */
export function deviceAction(d: DeviceRow): { kind: "linked" | "attach" | "create"; label: string } {
  if (d.linked) return { kind: "linked", label: d.printer_name ?? "Linked" };
  if (d.printer_id != null) return { kind: "attach", label: `Attach to ${d.printer_name ?? "printer"}` };
  return { kind: "create", label: "Create printer" };
}

export function isSixDigits(s: string): boolean {
  return /^\d{6}$/.test(s.trim());
}
