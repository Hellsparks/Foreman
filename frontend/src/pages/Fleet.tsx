import { useCallback, useEffect, useState } from "react";
import { useOutletContext } from "react-router-dom";
import { api } from "../api/client";
import type { PrinterWithStatus } from "../api/client";
import type { PrinterStatus } from "../api/types/PrinterStatus";
import type { StatusMap, ConnectionMap } from "../shell/Shell";
import "./Fleet.css";

interface FleetContext {
  statuses: StatusMap;
  connections: ConnectionMap;
}

export default function Fleet() {
  const { statuses, connections } = useOutletContext<FleetContext>();
  const [printers, setPrinters] = useState<PrinterWithStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [showForm, setShowForm] = useState(false);

  const loadPrinters = useCallback(async () => {
    try {
      const list = await api.listPrinters();
      setPrinters(list);
    } catch (e) {
      console.error("Failed to load printers:", e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadPrinters();
  }, [loadPrinters]);

  const handleAdd = async (name: string, url: string) => {
    try {
      await api.createPrinter({
        name,
        firmware: "moonraker",
        primary_url: url,
        api_key: null,
        serial_number: null,
        access_code: null,
        tags: [],
        hardened_nozzle: false,
        sort_order: null,
      });
      setShowForm(false);
      loadPrinters();
    } catch (e) {
      console.error("Failed to add printer:", e);
    }
  };

  const handleDelete = async (id: number) => {
    try {
      await api.deletePrinter(id);
      loadPrinters();
    } catch (e) {
      console.error("Failed to delete printer:", e);
    }
  };

  return (
    <div>
      <div className="page-header">
        <p className="page-header__eyebrow">Fleet</p>
        <h1 className="page-header__title">Printer fleet</h1>
      </div>

      <div className="fleet__actions">
        <button
          className="fleet__add-btn"
          onClick={() => setShowForm((s) => !s)}
        >
          {showForm ? "Cancel" : "+ Add printer"}
        </button>
      </div>

      {showForm && <AddPrinterForm onSubmit={handleAdd} />}

      {loading ? (
        <p className="fleet__loading">Loading printers...</p>
      ) : printers.length === 0 ? (
        <p className="fleet__empty">
          No printers registered. Add one to get started.
        </p>
      ) : (
        <div className="fleet__grid">
          {printers.map((p) => (
            <PrinterCard
              key={p.id}
              printer={p}
              liveStatus={statuses[p.id]}
              connectionState={connections[p.id]}
              onDelete={() => handleDelete(p.id)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Printer card
// ---------------------------------------------------------------------------

function PrinterCard({
  printer,
  liveStatus,
  connectionState,
  onDelete,
}: {
  printer: PrinterWithStatus;
  liveStatus?: PrinterStatus;
  connectionState?: string;
  onDelete: () => void;
}) {
  // Use live status from WS if available, fall back to REST snapshot.
  const status = liveStatus ?? printer.status;
  const conn = connectionState ?? (status ? "online" : "offline");

  const stateLabel = status?.state ?? "offline";
  const nozzle = status?.nozzle;
  const bed = status?.bed;
  const progress = status?.progress;

  return (
    <div className={`fleet-card fleet-card--${stateLabel}`}>
      <div className="fleet-card__header">
        <span className="fleet-card__name">{printer.name}</span>
        <span className={`fleet-card__conn fleet-card__conn--${conn}`}>
          {conn}
        </span>
      </div>

      <div className="fleet-card__state">{stateLabel}</div>

      <div className="fleet-card__temps">
        <div className="fleet-card__temp">
          <span className="fleet-card__temp-label">Nozzle</span>
          <span className="fleet-card__temp-value">
            {nozzle ? `${nozzle.actual.toFixed(0)}°` : "--"}
            {nozzle && nozzle.target > 0 && (
              <span className="fleet-card__temp-target">
                / {nozzle.target.toFixed(0)}°
              </span>
            )}
          </span>
        </div>
        <div className="fleet-card__temp">
          <span className="fleet-card__temp-label">Bed</span>
          <span className="fleet-card__temp-value">
            {bed ? `${bed.actual.toFixed(0)}°` : "--"}
            {bed && bed.target > 0 && (
              <span className="fleet-card__temp-target">
                / {bed.target.toFixed(0)}°
              </span>
            )}
          </span>
        </div>
      </div>

      {progress != null && progress > 0 && (
        <div className="fleet-card__progress">
          <div className="fleet-card__progress-bar">
            <div
              className="fleet-card__progress-fill"
              style={{ width: `${(progress * 100).toFixed(1)}%` }}
            />
          </div>
          <span className="fleet-card__progress-text">
            {(progress * 100).toFixed(0)}%
          </span>
        </div>
      )}

      {status?.filename && (
        <div className="fleet-card__filename">{status.filename}</div>
      )}

      <button className="fleet-card__delete" onClick={onDelete} title="Remove printer">
        &times;
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Add printer form
// ---------------------------------------------------------------------------

function AddPrinterForm({
  onSubmit,
}: {
  onSubmit: (name: string, url: string) => void;
}) {
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (name.trim() && url.trim()) {
      onSubmit(name.trim(), url.trim());
    }
  };

  return (
    <form className="fleet__form" onSubmit={handleSubmit}>
      <div className="fleet__form-field">
        <label className="fleet__form-label" htmlFor="printer-name">
          Name
        </label>
        <input
          id="printer-name"
          className="fleet__form-input"
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="e.g. HellV0"
          required
        />
      </div>
      <div className="fleet__form-field">
        <label className="fleet__form-label" htmlFor="printer-url">
          Moonraker URL
        </label>
        <input
          id="printer-url"
          className="fleet__form-input"
          type="url"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="http://10.0.0.177"
          required
        />
      </div>
      <button className="fleet__form-submit" type="submit">
        Add
      </button>
    </form>
  );
}
