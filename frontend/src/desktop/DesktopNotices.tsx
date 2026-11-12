/** Desktop-app strip under the top bar: version mismatch with the server, update available. */
import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import Button from "../ui/Button";
import { desktop } from "./bridge";
import { useRoute } from "./route";
import { useUpdate } from "./updates";
import { versionMismatch, type Mismatch } from "./versionCompat";
import "./Desktop.css";

export default function DesktopNotices() {
  const route = useRoute();
  const update = useUpdate();
  const navigate = useNavigate();
  const [mismatch, setMismatch] = useState<Mismatch | null>(null);
  const [hideMismatch, setHideMismatch] = useState(false);
  const [hideUpdate, setHideUpdate] = useState(false);
  const address = route?.address;

  useEffect(() => {
    let live = true;
    setMismatch(null);
    setHideMismatch(false);
    void Promise.all([desktop.appInfo(), api.serverInfo()])
      .then(([app, srv]) => {
        if (live) setMismatch(versionMismatch(app.version, srv.version));
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [address]);

  const review = () => navigate("/settings#about-heading");

  return (
    <>
      {mismatch && !hideMismatch && (
        <div className="dk-notice dk-notice--warn" role="status">
          <span className="dk-notice__text">{mismatch.message}</span>
          {mismatch.update === "app" && (
            <Button variant="small" onClick={review}>
              Settings → About
            </Button>
          )}
          <button type="button" className="dk-notice__close" aria-label="Dismiss version warning" onClick={() => setHideMismatch(true)}>
            ✕
          </button>
        </div>
      )}
      {update.status === "available" && !hideUpdate && (
        <div className="dk-notice" role="status">
          <span className="dk-notice__text">Foreman {update.info.version} is available.</span>
          <Button variant="small" onClick={review}>
            See what's new
          </Button>
          <button type="button" className="dk-notice__close" aria-label="Dismiss update notice" onClick={() => setHideUpdate(true)}>
            ✕
          </button>
        </div>
      )}
    </>
  );
}
