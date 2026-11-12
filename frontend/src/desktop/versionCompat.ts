/** App ↔ server version check (warn, never block). */

export interface Mismatch {
  /** Which side to update: the older one. */
  update: "app" | "server";
  message: string;
}

function parse(v: string): [number, number, number] | null {
  const m = /^v?(\d+)\.(\d+)\.(\d+)/.exec(v.trim());
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

/**
 * Null when the versions match or differ only in the patch number (or can't be read).
 * Otherwise says which side is older and should be updated.
 */
export function versionMismatch(appVersion: string, serverVersion: string): Mismatch | null {
  const a = parse(appVersion);
  const s = parse(serverVersion);
  if (!a || !s) return null;
  if (a[0] === s[0] && a[1] === s[1]) return null;
  const serverOlder = s[0] < a[0] || (s[0] === a[0] && s[1] < a[1]);
  if (serverOlder) {
    return {
      update: "server",
      message:
        `The server runs Foreman ${serverVersion}, older than this app (${appVersion}). ` +
        `Update the server to ${a[0]}.${a[1]} — some screens may not work until it matches.`,
    };
  }
  return {
    update: "app",
    message:
      `This app is Foreman ${appVersion}, older than the server (${serverVersion}). ` +
      `Update this app in Settings → About — some screens may not work until it matches.`,
  };
}
