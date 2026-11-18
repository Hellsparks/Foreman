/** Bambu Cloud accounts + sign-in (doc 07 §B). The token never reaches the browser. */
import { request } from "./client";
import type { AccountState } from "./types/AccountState";
import type { BambuAccountView } from "./types/BambuAccountView";
import type { CloudDevice } from "./types/CloudDevice";
import type { CodeChannel } from "./types/CodeChannel";
import type { CreateBambuAccount } from "./types/CreateBambuAccount";
import type { DeviceList } from "./types/DeviceList";
import type { Region } from "./types/Region";

export type { AccountState, BambuAccountView, CloudDevice, CodeChannel, CreateBambuAccount, Region };

const base = "/api/bambu/accounts";

export const bambuApi = {
  list: () => request<BambuAccountView[]>("GET", base),
  create: (body: CreateBambuAccount) => request<BambuAccountView>("POST", base, body),
  remove: (id: number) => request<void>("DELETE", `${base}/${id}`),
  signin: (id: number, password: string) => request<BambuAccountView>("POST", `${base}/${id}/signin`, { password }),
  code: (id: number, code: string) => request<BambuAccountView>("POST", `${base}/${id}/code`, { code }),
  token: (id: number, token: string) => request<BambuAccountView>("POST", `${base}/${id}/token`, { token }),
  devices: (id: number) => request<DeviceList>("GET", `${base}/${id}/devices`).then((d) => d.devices),
};
