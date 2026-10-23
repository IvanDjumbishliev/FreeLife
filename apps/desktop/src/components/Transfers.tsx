import type { Direction } from "../lib/api";
import { formatBytes, formatEta, formatSpeed } from "../lib/format";

export type TransferStatus = "active" | "done" | "failed";

export interface TransferView {
  id: string;
  filename: string;
  direction: Direction;
  peerName: string;
  bytes: number;
  total: number;
  speedBps: number;
  etaSecs: number | null;
  status: TransferStatus;
  error?: string;
  path?: string | null;
}

export function Transfers({ transfers }: { transfers: TransferView[] }) {
  if (transfers.length === 0) {
    return <p className="muted">No transfers yet.</p>;
  }
  return (
    <ul className="transfers">
      {transfers.map((t) => {
        const pct = t.total === 0 ? 100 : Math.floor((t.bytes / t.total) * 100);
        const arrow = t.direction === "send" ? "→" : "←";
        return (
          <li key={t.id} className={`transfer transfer-${t.status}`}>
            <div className="row">
              <span className="filename" title={t.path ?? t.filename}>
                {t.filename}
              </span>
              <span className="muted">
                {arrow} {t.peerName}
              </span>
            </div>
            <progress max={100} value={pct} />
            <div className="row muted small">
              <span>
                {formatBytes(t.bytes)} / {formatBytes(t.total)} ({pct}%)
              </span>
              <span>
                {t.status === "active" && `${formatSpeed(t.speedBps)} · ${formatEta(t.etaSecs)}`}
                {t.status === "done" && (t.direction === "receive" ? "Saved" : "Sent")}
                {t.status === "failed" && <span className="error">{t.error}</span>}
              </span>
            </div>
          </li>
        );
      })}
    </ul>
  );
}
