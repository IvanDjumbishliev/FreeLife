import type { HandoffEntry } from "../lib/api";

export function HandoffHistory({ entries }: { entries: HandoffEntry[] }) {
  return (
    <section>
      <h2>Handoff History</h2>
      {entries.length === 0 ? (
        <p className="muted">No links handed off yet.</p>
      ) : (
        <ul className="handoff-history">
          {entries.map((entry, index) => (
            <li key={`${entry.timestamp}:${entry.sourceDeviceId}:${index}`}>
              <a href={entry.url} target="_blank" rel="noreferrer">
                {entry.url}
              </a>
              <span className="muted">
                {new Date(entry.timestamp).toLocaleString()} · from {entry.sourceDeviceName}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
