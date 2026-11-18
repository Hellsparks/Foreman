import { describe, expect, it } from "vitest";
import type { BambuAccountView } from "../api/bambu";
import { canListDevices, codePrompt, isSixDigits, removeWarning, stateChip } from "./bambuLogic";

function acct(p: Partial<BambuAccountView>): BambuAccountView {
  return {
    id: 1, label: "Home", region: "global", email: "a@example.com", has_token: false,
    token_expires_at: null, last_ok_at: null, last_error: null, printer_count: 0,
    created_at: "2026-11-18T00:00:00Z", state: "signed_out", days_left: null, code_channel: null,
    ...p,
  };
}

describe("bambu account state chip", () => {
  it("connected shows the expiry date, never a token", () => {
    const c = stateChip(acct({ state: "connected", has_token: true, token_expires_at: "2027-02-16T12:00:00Z" }));
    expect(c.word).toBe("Connected");
    expect(c.tone).toBe("ok");
    expect(c.detail).toMatch(/^until /);
  });
  it("expiring counts days", () => {
    expect(stateChip(acct({ state: "expiring", days_left: 3 })).word).toBe("Expires in 3 days");
    expect(stateChip(acct({ state: "expiring", days_left: 1 })).word).toBe("Expires in 1 day");
  });
  it("error carries the message", () => {
    const c = stateChip(acct({ state: "error", last_error: "Incorrect password (HTTP 400)" }));
    expect(c.tone).toBe("bad");
    expect(c.detail).toBe("Incorrect password (HTTP 400)");
  });
  it("one code form, two prompts", () => {
    expect(codePrompt("email")).toMatch(/emailed/);
    expect(codePrompt("authenticator")).toMatch(/authenticator/);
  });
});

describe("remove warning names the printer count", () => {
  it.each([
    [0, "No printers reference this account."],
    [1, "1 printer references this account. It will be unlinked from Bambu Cloud (LAN settings are kept)."],
    [3, "3 printers reference this account. They will be unlinked from Bambu Cloud (LAN settings are kept)."],
  ])("%i", (n, text) => {
    expect(removeWarning(acct({ printer_count: n }))).toBe(text);
  });
});

it("device list only with a live token", () => {
  expect(canListDevices(acct({ has_token: true, state: "connected" }))).toBe(true);
  expect(canListDevices(acct({ has_token: true, state: "expired" }))).toBe(false);
  expect(canListDevices(acct({ has_token: false, state: "error" }))).toBe(false);
});

it("six digits", () => {
  expect(isSixDigits("123456")).toBe(true);
  expect(isSixDigits(" 123456 ")).toBe(true);
  expect(isSixDigits("12345")).toBe(false);
  expect(isSixDigits("12a456")).toBe(false);
});
