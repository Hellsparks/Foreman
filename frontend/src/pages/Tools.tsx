/** Tools (W10, W11). Reference: docs/ui-ref/Tools.dc.html. Calibration is live (5.4); the rest comes later. */
import { useState } from "react";
import Calibration from "../calibration/Calibration";
import "./Tools.css";

type Tab = "cal" | "swatch" | "import" | "export";
const TABS: [Tab, string][] = [
  ["cal", "Calibration"],
  ["swatch", "Swatches"],
  ["import", "Import"],
  ["export", "Export"],
];

export default function Tools() {
  const [tab, setTab] = useState<Tab>("cal");
  return (
    <div className="tl">
      <header className="tl-head">
        <h1 className="tl-title">Tools</h1>
        <div role="tablist" aria-label="Tools" className="tl-tabs">
          {TABS.map(([id, label]) => (
            <button key={id} type="button" role="tab" aria-selected={tab === id} className="tl-tab" onClick={() => setTab(id)}>
              {label}
            </button>
          ))}
        </div>
      </header>
      {tab === "cal" ? <Calibration /> : <p className="tl-muted">Coming in a later session.</p>}
    </div>
  );
}
