/** Settings → Setup: run the first-run wizard again (6.4). */
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { setupApi } from "../api/setup";
import { Button } from "../ui";

export default function RerunSetup() {
  const navigate = useNavigate();
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  return (
    <section aria-labelledby="setup-heading">
      <h2 id="setup-heading" style={{ font: "500 18px/1.3 var(--fw-sans)", margin: "0 0 var(--fw-space-4)", letterSpacing: "-0.02em" }}>
        Setup
      </h2>
      <p className="auth__hint" style={{ margin: "0 0 var(--fw-space-3)" }}>
        Walk through filament data, printers, slicer and backups again. Nothing changes until you confirm a step.
      </p>
      <Button
        variant="secondary"
        disabled={busy}
        onClick={async () => {
          setBusy(true);
          setErr(null);
          try {
            await setupApi.restart();
            navigate("/setup");
          } catch (e) {
            setErr(e instanceof Error ? e.message : String(e));
            setBusy(false);
          }
        }}
      >
        Run setup again
      </Button>
      {err && (
        <p role="alert" style={{ color: "var(--fw-danger)", fontSize: 13 }}>
          {err}
        </p>
      )}
    </section>
  );
}
