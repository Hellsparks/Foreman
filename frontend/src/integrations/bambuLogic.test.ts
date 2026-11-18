import { describe, expect, it } from "vitest";
import type { BambuAccountView } from "../api/bambu";
import { canListDevices, codePrompt, deviceAction, isSixDigits, removeWarning, stateChip } from "./bambuLogic";
import type { DeviceRow } from "../api/bambu";

function dev(p: Partial<DeviceRow>): DeviceRow {
  return {
    dev_id: "03919C461200718", name: "A1", online: true, dev_model_name: "N2S", dev_product_name: "A1",
    print_status: null, nozzle_diameter: 0.4, printer_id: null, printer_name: null, linked: false, feed: null, ...p,
  };
}

describe("bambu device rows", () => {
  it("offers attach when a printer has the serial, create otherwise, and names a linked one", () => {
    expect(deviceAction(dev({})).kind).toBe("create");
    expect(deviceAction(dev({ printer_id: 4, printer_name: "bambu" }))).toEqual({ kind: "attach", label: "Attach to bambu" });
    expect(deviceAction(dev({ printer_id: 4, printer_name: "bambu", linked: true }))).toEqual({ kind: "linked", label: "bambu" });
  });
});

describe("bambu expired by rejection", () => {
  it("says Bambu refused the sign-in", () => {
    const c = stateChip(acct({ state: "expired", has_token: true, token_rejected_at: "2026-11-18T12:00:00Z" }));
    expect(c.word).toBe("Expired");
    expect(c.detail).toMatch(/^Bambu refused the sign-in on /);
  });
});

function acct(p: Partial<BambuAccountView>): BambuAccountView {
  return {
    id: 1, label: "Home", region: "global", email: "a@example.com", has_token: false,
    token_expires_at: null, last_ok_at: null, last_error: null, token_rejected_at: null, printer_count: 0,
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
