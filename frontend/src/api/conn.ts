/**
 * Where API calls go.
 *
 * Browser: same origin as the page, session cookie (base "" and no token).
 * Desktop app: an explicit server address and a bearer token per saved server.
 */

interface Conn {
  base: string;
  token: string | null;
}

let conn: Conn = { base: "", token: null };
const listeners = new Set<() => void>();

export function getConnection(): Conn {
  return conn;
}

export function setConnection(base: string, token: string | null): void {
  if (conn.base === base && conn.token === token) return;
  conn = { base, token };
  listeners.forEach((l) => l());
}

/** Subscribe to base/token changes (e.g. failover switched address). */
export function onConnectionChange(cb: () => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

/** `10.0.0.20:9090` → `http://10.0.0.20:9090` (keeps an explicit https://). */
export function baseFor(address: string): string {
  return /^https?:\/\//.test(address) ? address : `http://${address}`;
}

export function apiUrl(path: string): string {
  return conn.base + path;
}

export function authHeaders(): Record<string, string> {
  return conn.token ? { Authorization: `Bearer ${conn.token}` } : {};
}

function withToken(url: string): string {
  if (!conn.token) return url;
  return `${url}${url.includes("?") ? "&" : "?"}token=${encodeURIComponent(conn.token)}`;
}

/** For `<img src>` / `<a href>`: these can't send headers, so the token rides in the query. */
export function mediaUrl(path: string): string {
  return withToken(apiUrl(path));
}

export function wsUrl(path: string): string {
  const httpBase = conn.base || `${window.location.protocol}//${window.location.host}`;
  return withToken(httpBase.replace(/^http/, "ws") + path);
}
