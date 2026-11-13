/** Desktop-app (Tauri) commands. Only call these when `IS_TAURI`. */
import { invoke } from "@tauri-apps/api/core";
import type { ReportedAddress } from "../api/client";

export interface Found {
  id: string;
  name: string;
  version: string;
  host: string;
  needs_setup: boolean;
  addresses: string[];
}

export type Probe =
  | {
      kind: "foreman";
      address: string;
      server_id: string;
      name: string;
      version: string;
      needs_setup: boolean;
      addresses: ReportedAddress[];
    }
  | { kind: "spoolman"; address: string; suggestion: string }
  | { kind: "moonraker"; address: string; hostname: string | null }
  | { kind: "no_answer"; address: string; error: string }
  | { kind: "invalid"; error: string };

export interface SavedServer {
  id: string;
  name: string;
  version?: string;
  /** In order: primary first, then backups. */
  addresses: string[];
  token: string | null;
  last_used?: number;
}

export interface DesktopStore {
  /** Server id to reopen on launch, `"local"` for Mode A, or null for the start screen. */
  last?: string | null;
  servers?: SavedServer[];
  local?: { token?: string | null };
}

export interface LocalConfig {
  port: number;
  spoolman_port: number | null;
  data_dir: string;
}

export interface LocalStarted {
  address: string;
  setup_code: string | null;
}

export interface DesktopPrefs {
  /** Effective "keep running in the background". */
  keep_running: boolean;
  /** The stored choice; null = default (on while the built-in server is in use). */
  keep_running_choice: boolean | null;
  autostart: boolean;
  local_in_use: boolean;
  tray_available: boolean;
  /** "beta" also offers pre-releases (v0.4.0-rc.1). */
  update_channel: UpdateChannel;
}

export type UpdateChannel = "stable" | "beta";

export interface AppInfo {
  version: string;
  commit: string;
  app_data_dir: string;
  server_data_dir: string;
}

export interface UpdateInfo {
  version: string;
  current_version: string;
  notes: string | null;
  date: string | null;
}

export const desktop = {
  loadStore: () => invoke<DesktopStore>("store_load"),
  saveStore: (doc: DesktopStore) => invoke<void>("store_save", { doc }),
  discover: (timeoutMs = 2500) => invoke<Found[]>("discover_servers", { timeoutMs }),
  probe: (address: string) => invoke<Probe>("probe_address", { address }),
  /** Server id answering at `address`, or null. */
  check: (address: string) => invoke<string | null>("check_address", { address }),
  localConfig: () => invoke<LocalConfig>("local_server_config"),
  startLocal: () => invoke<LocalStarted>("start_local_server"),
  localToken: () => invoke<string>("local_token"),
  prefs: () => invoke<DesktopPrefs>("desktop_prefs"),
  setKeepRunning: (on: boolean) => invoke<DesktopPrefs>("set_keep_running", { on }),
  setAutostart: (on: boolean) => invoke<DesktopPrefs>("set_autostart", { on }),
  setUpdateChannel: (channel: UpdateChannel) => invoke<DesktopPrefs>("set_update_channel", { channel }),
  appInfo: () => invoke<AppInfo>("app_info"),
  /** Asks the chosen update channel; null when this is the newest version. */
  updateCheck: () => invoke<UpdateInfo | null>("update_check"),
  /** Downloads, stops the built-in server, installs and restarts the app. */
  updateInstall: () => invoke<void>("update_install"),
};
