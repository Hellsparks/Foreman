/**
 * Empty Fleet (FleetEmpty.dc.html): the getting-started checklist + printers found on
 * the network, until the first printer is added. Someone who skipped the wizard
 * still has a path from here (6.4).
 */
import { useCallback, useEffect, useState } from "react";
import { setupApi, type SetupOverview } from "../api/setup";
import Checklist from "./Checklist";
import FoundPrinters from "./FoundPrinters";
import "./setup.css";

export default function FleetGettingStarted({ onAdded }: { onAdded: () => void }) {
  const [ov, setOv] = useState<SetupOverview | null>(null);
  const load = useCallback(async () => {
    try {
      setOv(await setupApi.get());
    } catch (e) {
      console.error("setup overview:", e);
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="su-fleet-empty">
      {ov && <Checklist ov={ov} mode="fleet" onChange={() => void load()} />}
      <FoundPrinters
        onAdded={() => {
          void load();
          onAdded();
        }}
      />
    </div>
  );
}
