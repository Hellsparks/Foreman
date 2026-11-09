/** Address helpers shared by the Connect screens and failover. */

/** `10.0.0.20` → `10.0.0.20:9090`; keeps an explicit port and https://. */
export function withPort(raw: string): string {
  const a = raw.trim().replace(/\/+$/, "");
  const hostPart = a.replace(/^https?:\/\//, "");
  return /:\d+$/.test(hostPart) ? a : `${a}:9090`;
}

export function sameAddress(a: string, b: string): boolean {
  const norm = (s: string) => withPort(s).replace(/^http:\/\//, "").toLowerCase();
  return norm(a) === norm(b);
}

function host(address: string): string {
  return address.replace(/^https?:\/\//, "").replace(/:\d+$/, "").replace(/^\[|\]$/g, "");
}

function ipv4(h: string): number[] | null {
  const m = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(h);
  return m ? m.slice(1).map(Number) : null;
}

/** Human label for how an address reaches the server ("LAN IP", "Tailscale", …). */
export function addressKind(address: string): string {
  const h = host(address).toLowerCase();
  if (h === "localhost" || h.startsWith("127.")) return "This computer";
  if (h.endsWith(".ts.net")) return "Tailscale";
  if (h.endsWith(".local")) return "LAN name";
  const ip = ipv4(h);
  if (ip) {
    if (ip[0] === 100 && (ip[1] & 0xc0) === 64) return "Tailscale IP";
    if (ip[0] === 10 || (ip[0] === 172 && ip[1] >= 16 && ip[1] <= 31) || (ip[0] === 192 && ip[1] === 168)) {
      return "LAN IP";
    }
    return "IP address";
  }
  return "Hostname";
}

const REPORTED_KIND: Record<string, string> = {
  loopback: "This computer",
  lan_ip: "LAN IP",
  lan_name: "LAN name",
  tailscale_ip: "Tailscale IP",
  tailscale_name: "Tailscale",
};

export function reportedKind(kind: string): string {
  return REPORTED_KIND[kind] ?? "Address";
}

export function timeAgo(unixMs: number | undefined): string {
  if (!unixMs) return "not used yet";
  const s = Math.max(0, Math.round((Date.now() - unixMs) / 1000));
  if (s < 90) return "last used just now";
  const m = Math.round(s / 60);
  if (m < 90) return `last used ${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 36) return `last used ${h} h ago`;
  const d = Math.round(h / 24);
  if (d < 14) return `last used ${d} days ago`;
  return `last used ${Math.round(d / 7)} weeks ago`;
}
