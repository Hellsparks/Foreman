/** Bambu Cloud accounts + sign-in (doc 07 §B). The token never reaches the browser. */
import { request } from "./client";
import type { AccountState } from "./types/AccountState";
import type { BambuAccountView } from "./types/BambuAccountView";
import type { CloudDevice } from "./types/CloudDevice";
import type { CodeChannel } from "./types/CodeChannel";
import type { CreateBambuAccount } from "./types/CreateBambuAccount";
import type { DeviceList } from "./types/DeviceList";
import type { DeviceRow } from "./types/DeviceRow";
import type { LinkResult } from "./types/LinkResult";
import type { CloudHistory } from "./types/CloudHistory";
import type { CloudTask } from "./types/CloudTask";
import type { Region } from "./types/Region";

export type {
  AccountState,
  BambuAccountView,
  CloudDevice,
  CloudHistory,
  CloudTask,
  CodeChannel,
  CreateBambuAccount,
  DeviceRow,
  LinkResult,
  Region,
};

const base = "/api/bambu/accounts";

export const bambuApi = {
  list: () => request<BambuAccountView[]>("GET", base),
  create: (body: CreateBambuAccount) => request<BambuAccountView>("POST", base, body),
  remove: (id: number) => request<void>("DELETE", `${base}/${id}`),
  signin: (id: number, password: string) => request<BambuAccountView>("POST", `${base}/${id}/signin`, { password }),
  code: (id: number, code: string) => request<BambuAccountView>("POST", `${base}/${id}/code`, { code }),
  token: (id: number, token: string) => request<BambuAccountView>("POST", `${base}/${id}/token`, { token }),
  devices: (id: number) => request<DeviceList>("GET", `${base}/${id}/devices`).then((d) => d.devices),
  /** Attach the device to the printer with its serial, or create one. */
  link: (id: number, serial: string) =>
    request<LinkResult>("POST", `${base}/${id}/devices/${encodeURIComponent(serial)}/link`),
  unlink: (id: number, serial: string) =>
    request<void>("DELETE", `${base}/${id}/devices/${encodeURIComponent(serial)}/link`),
  /** Read-only cloud print history of a linked printer. */
  history: (printerId: number) => request<CloudHistory>("GET", `/api/printers/${printerId}/cloud-history`),
};
