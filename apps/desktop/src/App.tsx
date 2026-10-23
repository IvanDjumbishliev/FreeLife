import { useEffect, useState } from "react";
import {
  discoverDevices,
  getDeviceInfo,
  sendClipboard,
  type DeviceInfo,
  type Peer,
} from "./lib/api";

export default function App() {
  const [self, setSelf] = useState<DeviceInfo | null>(null);
  const [peers, setPeers] = useState<Peer[]>([]);
  const [scanning, setScanning] = useState(false);
  const [clipboard, setClipboard] = useState("");
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    getDeviceInfo()
      .then(setSelf)
      .catch((e) => setStatus(String(e)));
  }, []);

  async function scan() {
    setScanning(true);
    setStatus(null);
    try {
      setPeers(await discoverDevices());
    } catch (e) {
      setStatus(String(e));
    } finally {
      setScanning(false);
    }
  }

  async function shareClipboard(peer: Peer) {
    try {
      await sendClipboard(peer.info.id, clipboard);
      setStatus(`Sent clipboard to ${peer.info.name}`);
    } catch (e) {
      setStatus(String(e));
    }
  }

  return (
    <main className="container">
      <header>
        <h1>FreeLife</h1>
        {self && (
          <p className="muted">
            This device: <strong>{self.name}</strong> ({self.platform}) — port {self.port}
          </p>
        )}
      </header>

      <section>
        <div className="row">
          <h2>Nearby devices</h2>
          <button onClick={scan} disabled={scanning}>
            {scanning ? "Scanning…" : "Scan"}
          </button>
        </div>
        {peers.length === 0 ? (
          <p className="muted">No devices found yet.</p>
        ) : (
          <ul className="peers">
            {peers.map((peer) => (
              <li key={peer.info.id}>
                <span>
                  {peer.info.name} <span className="muted">({peer.info.platform})</span>
                </span>
                <button onClick={() => shareClipboard(peer)} disabled={!clipboard}>
                  Send clipboard
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section>
        <h2>Clipboard</h2>
        <textarea
          value={clipboard}
          onChange={(e) => setClipboard(e.target.value)}
          placeholder="Text to send to a device…"
          rows={3}
        />
      </section>

      {status && <p className="status">{status}</p>}
    </main>
  );
}
