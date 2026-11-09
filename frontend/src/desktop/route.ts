import { createContext, useContext } from "react";

/** The desktop app's current server + route; `null` in the browser. */
export interface RouteInfo {
  mode: "local" | "remote";
  serverName: string;
  address: string;
  /** Label of the backup route in use ("Tailscale", "LAN name", …), or null on the primary. */
  via: string | null;
  switchServer: () => void;
}

export const RouteContext = createContext<RouteInfo | null>(null);

export function useRoute(): RouteInfo | null {
  return useContext(RouteContext);
}
