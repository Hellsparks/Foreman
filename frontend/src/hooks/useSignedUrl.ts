import { useEffect, useState } from "react";
import { apiUrl, getConnection, onConnectionChange, signedUrl } from "../api/conn";

const REFRESH_MS = 40_000;

/**
 * `path` → URL usable in `<img src>` / `<a href>`. In the desktop app it carries a scoped
 * 60 s URL token (refreshed so links stay valid); in the browser the cookie is enough.
 */
export function useSignedUrl(path: string | null): string | null {
  const [url, setUrl] = useState<string | null>(() =>
    path && !getConnection().token ? apiUrl(path) : null,
  );
  const [rev, setRev] = useState(0);

  useEffect(() => onConnectionChange(() => setRev((r) => r + 1)), []);

  useEffect(() => {
    if (!path) {
      setUrl(null);
      return;
    }
    if (!getConnection().token) {
      setUrl(apiUrl(path));
      return;
    }
    let live = true;
    const load = () =>
      void signedUrl(path)
        .then((u) => live && setUrl(u))
        .catch(() => {});
    load();
    const timer = setInterval(load, REFRESH_MS);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [path, rev]);

  return url;
}
