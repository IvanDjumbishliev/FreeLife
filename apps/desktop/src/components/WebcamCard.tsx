import { useEffect, useState } from "react";
import {
  onCameraEvent,
  phoneWebcamStart,
  phoneWebcamStatus,
  phoneWebcamStop,
  type CameraEvent,
  type PairedDevice,
} from "../lib/api";

interface Props {
  /** A connected paired phone, or null when none is available. */
  phone: PairedDevice | null;
  onError: (message: string) => void;
}

interface Stats {
  width: number;
  height: number;
  fps: number;
  latencyMs: number | null;
  framesTotal: number;
  decodeErrors: number;
  sink: string;
}

/** "Use Phone as Webcam": one click to start, live preview and stats while on. */
export function WebcamCard({ phone, onError }: Props) {
  const [activeId, setActiveId] = useState<string | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const [stats, setStats] = useState<Stats | null>(null);
  const [sinkError, setSinkError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    phoneWebcamStatus().then(setActiveId).catch(() => setActiveId(null));
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    onCameraEvent((e: CameraEvent) => {
      switch (e.type) {
        case "stats":
          setStats(e);
          setSinkError(null);
          break;
        case "preview":
          setPreview(e.pngBase64);
          break;
        case "sinkError":
          setSinkError(e.message);
          break;
        case "ended":
          setActiveId(null);
          setStats(null);
          setPreview(null);
          break;
        case "started":
          break;
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  if (!phone && !activeId) return null;

  async function start() {
    if (!phone) return;
    setBusy(true);
    try {
      await phoneWebcamStart(phone.device.id);
      setActiveId(phone.device.id);
      setSinkError(null);
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function stop() {
    setBusy(true);
    try {
      await phoneWebcamStop();
    } catch (e) {
      onError(String(e));
    } finally {
      setActiveId(null);
      setStats(null);
      setPreview(null);
      setBusy(false);
    }
  }

  const active = activeId !== null;
  const name = phone?.device.name ?? "your phone";

  return (
    <div className="webcam-card">
      <div className="row">
        <div>
          <strong>Use Phone as Webcam</strong>
          <div className="muted small">
            {active
              ? `Streaming ${name}'s rear camera to “FreeLife Camera”.`
              : `Use ${name}'s camera in video calls on this computer.`}
          </div>
        </div>
        {active ? (
          <button onClick={stop} disabled={busy}>
            Stop
          </button>
        ) : (
          <button className="primary" onClick={start} disabled={busy || !phone}>
            Use Phone as Webcam
          </button>
        )}
      </div>

      {active && (
        <div className="webcam-body">
          {preview ? (
            <img
              className="webcam-preview"
              alt="Phone camera preview"
              src={`data:image/png;base64,${preview}`}
            />
          ) : (
            <div className="webcam-preview placeholder muted small">Waiting for video…</div>
          )}
          {stats ? (
            <dl className="webcam-stats">
              <dt>Resolution</dt>
              <dd>
                {stats.width}×{stats.height}
              </dd>
              <dt>Frame rate</dt>
              <dd>{stats.fps.toFixed(1)} fps</dd>
              <dt>Latency</dt>
              <dd>{stats.latencyMs === null ? "—" : `${Math.round(stats.latencyMs)} ms`}</dd>
              <dt>Frames</dt>
              <dd>
                {stats.framesTotal}
                {stats.decodeErrors > 0 && (
                  <span className="muted small"> ({stats.decodeErrors} skipped)</span>
                )}
              </dd>
              <dt>Output</dt>
              <dd className="small">{stats.sink}</dd>
            </dl>
          ) : (
            <p className="muted small">Measuring…</p>
          )}
          {sinkError && <p className="error small">{sinkError}</p>}
        </div>
      )}
    </div>
  );
}
