/** Shared update-check state: checked once on launch, again from Settings → About. */
import { useSyncExternalStore } from "react";
import { desktop, type UpdateInfo } from "./bridge";

export type UpdateState =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "current"; checkedAt: number }
  | { status: "available"; info: UpdateInfo }
  | { status: "installing"; info: UpdateInfo }
  | { status: "error"; error: string; info?: UpdateInfo };

let state: UpdateState = { status: "idle" };
const subs = new Set<() => void>();

function set(next: UpdateState) {
  state = next;
  subs.forEach((f) => f());
}

export async function checkForUpdate(): Promise<void> {
  if (state.status === "checking" || state.status === "installing") return;
  set({ status: "checking" });
  try {
    const info = await desktop.updateCheck();
    set(info ? { status: "available", info } : { status: "current", checkedAt: Date.now() });
  } catch (e) {
    set({ status: "error", error: String(e) });
  }
}

export async function installUpdate(): Promise<void> {
  if (state.status !== "available") return;
  const info = state.info;
  set({ status: "installing", info });
  try {
    await desktop.updateInstall(); // restarts the app on success
  } catch (e) {
    set({ status: "error", error: String(e), info });
  }
}

export function useUpdate(): UpdateState {
  return useSyncExternalStore(
    (f) => {
      subs.add(f);
      return () => subs.delete(f);
    },
    () => state,
  );
}
