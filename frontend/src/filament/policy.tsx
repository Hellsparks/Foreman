/**
 * Filament tracking policy for the whole UI (doc 07 §A).
 *
 * The server decides what a mode allows (`FilamentPolicy`); components read the
 * named flags from `useFilament()` and never compare the mode themselves.
 * Until the policy has loaded, spool UI stays hidden so an install with
 * tracking Off never flashes it.
 */
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { Navigate } from "react-router-dom";
import { trackingApi, type FilamentPolicy, type FilamentSettings } from "../api/tracking";
import { useEventStream } from "../hooks/useEventStream";

export interface FilamentFeatures {
  /** The policy has loaded. */
  ready: boolean;
  policy: FilamentPolicy | null;
  /** Spools take part: Filament section, Spools tab, spool pickers, spool columns. */
  spools: boolean;
  /** Tools cards that need filament records (HueForge TD, swatches, spool exports). */
  filamentRecords: boolean;
  /** Re-read after a change in Settings. */
  reload: () => Promise<FilamentSettings | null>;
}

const Ctx = createContext<FilamentFeatures>({
  ready: false,
  policy: null,
  spools: false,
  filamentRecords: false,
  reload: async () => null,
});

/** Mode changes broadcast `queue` (queues re-read verdicts too). */
const TOPICS = ["queue"];

export function FilamentProvider({ children }: { children: ReactNode }) {
  const [policy, setPolicy] = useState<FilamentPolicy | null>(null);

  const reload = useCallback(async () => {
    try {
      const s = await trackingApi.get();
      setPolicy(s.policy);
      return s;
    } catch {
      return null;
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);
  useEventStream(
    useCallback(() => void reload(), [reload]),
    TOPICS,
  );

  const value = useMemo<FilamentFeatures>(
    () => ({
      ready: policy != null,
      policy,
      spools: policy?.spools ?? false,
      filamentRecords: policy?.spools ?? false,
      reload,
    }),
    [policy, reload],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useFilament(): FilamentFeatures {
  return useContext(Ctx);
}

/** Renders its children only while spools are tracked; otherwise sends the
 *  user back to the fleet (a bookmarked /filament with tracking Off). */
export function RequireSpools({ children }: { children: ReactNode }) {
  const { ready, spools } = useFilament();
  if (!ready) return null;
  return spools ? <>{children}</> : <Navigate to="/fleet" replace />;
}
