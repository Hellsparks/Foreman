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
/** Scoped URL tokens by path; the server keeps them for 60 s, reuse for 45 s. */
const urlTokens = new Map<string, { token: string; until: number }>();
const URL_TOKEN_REUSE_MS = 45_000;

export function getConnection(): Conn {
  return conn;
}

export function setConnection(base: string, token: string | null): void {
  if (conn.base === base && conn.token === token) return;
  conn = { base, token };
  urlTokens.clear();
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

function withToken(url: string, token: string | null): string {
  if (!token) return url;
  return `${url}${url.includes("?") ? "&" : "?"}token=${encodeURIComponent(token)}`;
}

/**
 * A 60 s token scoped to one snapshot/download/WebSocket path (desktop app only;
 * the browser's cookie already covers these). Session tokens never go in URLs.
 */
async function urlToken(path: string): Promise<string | null> {
  if (!conn.token) return null;
  const pathname = path.split("?")[0];
  const hit = urlTokens.get(pathname);
  if (hit && hit.until > Date.now()) return hit.token;
  const res = await fetch(apiUrl("/api/auth/url-token"), {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaders() },
    body: JSON.stringify({ path: pathname }),
  });
  if (!res.ok) return null;
  const { token } = (await res.json()) as { token: string };
  urlTokens.set(pathname, { token, until: Date.now() + URL_TOKEN_REUSE_MS });
  return token;
}

/** For `<img src>` / `<a href>`, which can't send headers. */
export async function signedUrl(path: string): Promise<string> {
  return withToken(apiUrl(path), await urlToken(path));
}

export async function wsUrl(path: string): Promise<string> {
  const httpBase = conn.base || `${window.location.protocol}//${window.location.host}`;
  return withToken(httpBase.replace(/^http/, "ws") + path, await urlToken(path));
}
