import { useCallback, useEffect, useState } from "react";
import { useOutletContext } from "react-router-dom";
import { api } from "../api/client";
import type { PrinterWithStatus } from "../api/client";
import type { FirmwareKind } from "../api/types/FirmwareKind";
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

  const handleAdd = async (data: AddPrinterData) => {
    try {
      await api.createPrinter({
        name: data.name,
        firmware: data.firmware,
        primary_url: data.url || null,
        api_key: null,
        serial_number: data.serial || null,
        access_code: data.accessCode || null,
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

      {status?.ams_trays && status.ams_trays.length > 0 && (
        <div className="fleet-card__ams">
          {status.ams_trays
            .filter((t) => t.slot < 254)
            .map((tray) => (
              <div
                key={tray.slot}
                className="fleet-card__ams-swatch"
                style={{
                  backgroundColor: tray.color ? `#${tray.color}` : "var(--fw-muted)",
                }}
                title={`Slot ${tray.slot}: ${tray.material ?? "?"} ${tray.remaining != null ? `${tray.remaining}%` : ""}`}
              />
            ))}
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

interface AddPrinterData {
  name: string;
  firmware: FirmwareKind;
  url: string;
  serial: string;
  accessCode: string;
}

function AddPrinterForm({
  onSubmit,
}: {
  onSubmit: (data: AddPrinterData) => void;
}) {
  const [name, setName] = useState("");
  const [firmware, setFirmware] = useState<FirmwareKind>("moonraker");
  const [url, setUrl] = useState("");
  const [serial, setSerial] = useState("");
  const [accessCode, setAccessCode] = useState("");

  const isBambu = firmware === "bambu";

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    if (isBambu) {
      if (!url.trim() || !serial.trim() || !accessCode.trim()) return;
    } else {
      if (!url.trim()) return;
    }
    onSubmit({
      name: name.trim(),
      firmware,
      url: url.trim(),
      serial: serial.trim(),
      accessCode: accessCode.trim(),
    });
  };

  return (
    <form className="fleet__form fleet__form--multi" onSubmit={handleSubmit}>
      <div className="fleet__form-row">
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
        <div className="fleet__form-field fleet__form-field--short">
          <label className="fleet__form-label" htmlFor="printer-firmware">
            Type
          </label>
          <select
            id="printer-firmware"
            className="fleet__form-input"
            value={firmware}
            onChange={(e) => setFirmware(e.target.value as FirmwareKind)}
          >
            <option value="moonraker">Moonraker</option>
            <option value="bambu">Bambu</option>
          </select>
        </div>
      </div>
      <div className="fleet__form-row">
        <div className="fleet__form-field">
          <label className="fleet__form-label" htmlFor="printer-url">
            {isBambu ? "IP Address" : "Moonraker URL"}
          </label>
          <input
            id="printer-url"
            className="fleet__form-input"
            type={isBambu ? "text" : "url"}
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder={isBambu ? "10.0.0.249" : "http://10.0.0.177"}
            required
          />
        </div>
        {isBambu && (
          <>
            <div className="fleet__form-field">
              <label className="fleet__form-label" htmlFor="printer-serial">
                Serial
              </label>
              <input
                id="printer-serial"
                className="fleet__form-input"
                type="text"
                value={serial}
                onChange={(e) => setSerial(e.target.value)}
                placeholder="e.g. 00W00A000000000"
                required
              />
            </div>
            <div className="fleet__form-field">
              <label className="fleet__form-label" htmlFor="printer-access">
                Access code
              </label>
              <input
                id="printer-access"
                className="fleet__form-input"
                type="password"
                value={accessCode}
                onChange={(e) => setAccessCode(e.target.value)}
                placeholder="8-digit code"
                required
              />
            </div>
          </>
        )}
        <button className="fleet__form-submit" type="submit">
          Add
        </button>
      </div>
    </form>
  );
}
