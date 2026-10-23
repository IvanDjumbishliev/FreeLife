import { useCallback, useEffect, useRef, useState } from "react";
import {
  discoverDevices,
  getDeviceInfo,
  listPairedDevices,
  onClipboardReceived,
  onPairingDismiss,
  onPairingPrompt,
  onTransferEvent,
  pairDevice,
  respondPairing,
  sendClipboard,
  sendFile,
  unpairDevice,
  type DeviceInfo,
  type NearbyDevice,
  type PairedDevice,
  type PairingPrompt,
  type TransferEvent,
} from "./lib/api";
import { basename } from "./lib/format";
import { useTauriEvent } from "./lib/useTauriEvent";
import { DropZone } from "./components/DropZone";
import { PairingDialog } from "./components/PairingDialog";
import { Toasts, useToasts } from "./components/Toasts";
import { Transfers, type TransferView } from "./components/Transfers";

export default function App() {
  const [self, setSelf] = useState<DeviceInfo | null>(null);
  const [paired, setPaired] = useState<PairedDevice[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [nearby, setNearby] = useState<NearbyDevice[]>([]);
  const [scanning, setScanning] = useState(false);
  const [pairing, setPairing] = useState<string | null>(null);
  const [prompts, setPrompts] = useState<PairingPrompt[]>([]);
  const [transfers, setTransfers] = useState<TransferView[]>([]);
  const [clipboard, setClipboard] = useState("");
  const toasts = useToasts();
  const { push } = toasts;

  // Names for transfer rows; a ref so the event handler sees current data.
  const names = useRef(new Map<string, string>());
  useEffect(() => {
    for (const p of paired) names.current.set(p.device.id, p.device.name);
  }, [paired]);
  const nameOf = (id: string) => names.current.get(id) ?? "unknown device";

  const refreshPaired = useCallback(async () => {
    try {
      const list = await listPairedDevices();
      setPaired(list);
      setSelected((s) => (s && list.some((p) => p.device.id === s) ? s : list[0]?.device.id ?? null));
    } catch (e) {
      push(String(e), "error");
    }
  }, [push]);

  useEffect(() => {
    getDeviceInfo().then(setSelf).catch((e) => push(String(e), "error"));
    refreshPaired();
    // Connection state changes aren't evented yet, so poll gently.
    const timer = setInterval(refreshPaired, 5000);
    return () => clearInterval(timer);
  }, [refreshPaired, push]);

  useTauriEvent(() =>
    onTransferEvent((e: TransferEvent) => {
      setTransfers((list) => applyTransferEvent(list, e, nameOf(e.peer)));
      if (e.type === "transferComplete") {
        push(
          e.direction === "send"
            ? `Sent ${e.filename} to ${nameOf(e.peer)}`
            : `Received ${e.filename} from ${nameOf(e.peer)}`,
          "success",
        );
      } else if (e.type === "transferError") {
        push(`${e.filename}: ${e.error}`, "error");
      }
    }),
  );
  useTauriEvent(() => onPairingPrompt((p) => setPrompts((list) => [...list, p])));
  useTauriEvent(() =>
    onPairingDismiss((id) => {
      setPrompts((list) => list.filter((p) => p.requestId !== id));
      refreshPaired();
    }),
  );
  useTauriEvent(() =>
    onClipboardReceived((c) => {
      setClipboard(c.text);
      push(`Clipboard received from ${nameOf(c.from)}`, "info");
    }),
  );

  const selectedDevice = paired.find((p) => p.device.id === selected)?.device ?? null;

  const handleDrop = useCallback(
    (paths: string[]) => {
      if (!selected) {
        push("Select a paired device first", "error");
        return;
      }
      for (const path of paths) {
        // Errors are also reported via transfer events; this catches
        // failures before the transfer starts (e.g. device unreachable).
        sendFile(selected, path).catch((e) => push(`${basename(path)}: ${e}`, "error"));
      }
    },
    [selected, push],
  );

  async function scan() {
    setScanning(true);
    try {
      setNearby(await discoverDevices());
    } catch (e) {
      push(String(e), "error");
    } finally {
      setScanning(false);
    }
  }

  async function pair(device: DeviceInfo) {
    setPairing(device.id);
    try {
      const peer = await pairDevice(device.id);
      push(`Paired with ${peer.name}`, "success");
      setNearby((list) => list.map((n) => (n.device.id === peer.id ? { ...n, paired: true } : n)));
      await refreshPaired();
      setSelected(peer.id);
    } catch (e) {
      push(`Pairing failed: ${e}`, "error");
    } finally {
      setPairing(null);
    }
  }

  async function unpair(device: DeviceInfo) {
    await unpairDevice(device.id).catch((e) => push(String(e), "error"));
    refreshPaired();
  }

  async function answerPrompt(prompt: PairingPrompt, accept: boolean) {
    setPrompts((list) => list.filter((p) => p.requestId !== prompt.requestId));
    await respondPairing(prompt.requestId, accept);
  }

  async function shareClipboard() {
    if (!selected) return;
    try {
      await sendClipboard(selected, clipboard);
      push(`Clipboard sent to ${selectedDevice?.name}`, "success");
    } catch (e) {
      push(String(e), "error");
    }
  }

  return (
    <main className="container">
      <header>
        <h1>FreeLife</h1>
        {self && (
          <p className="muted">
            This device: <strong>{self.name}</strong> ({self.os}) · key {self.fingerprint}
          </p>
        )}
      </header>

      <section>
        <h2>Paired devices</h2>
        {paired.length === 0 ? (
          <p className="muted">No paired devices yet. Scan below to pair one.</p>
        ) : (
          <ul className="devices">
            {paired.map(({ device, connected }) => (
              <li key={device.id}>
                <label>
                  <input
                    type="radio"
                    name="target"
                    checked={selected === device.id}
                    onChange={() => setSelected(device.id)}
                  />
                  <span className={`dot ${connected ? "dot-on" : ""}`} title={connected ? "Connected" : "Not connected"} />
                  {device.name} <span className="muted">({device.os})</span>
                </label>
                <button className="link" onClick={() => unpair(device)}>
                  Unpair
                </button>
              </li>
            ))}
          </ul>
        )}
        <DropZone target={selectedDevice?.name ?? null} onDrop={handleDrop} />
      </section>

      <section>
        <h2>Transfers</h2>
        <Transfers transfers={transfers} />
      </section>

      <section>
        <div className="row">
          <h2>Nearby devices</h2>
          <button onClick={scan} disabled={scanning}>
            {scanning ? "Scanning…" : "Scan"}
          </button>
        </div>
        {nearby.length === 0 ? (
          <p className="muted">No devices found yet.</p>
        ) : (
          <ul className="devices">
            {nearby.map(({ device, paired: isPaired }) => (
              <li key={device.id}>
                <span>
                  {device.name} <span className="muted">({device.os})</span>
                </span>
                {isPaired ? (
                  <span className="muted">Paired</span>
                ) : (
                  <button onClick={() => pair(device)} disabled={pairing !== null}>
                    {pairing === device.id ? "Pairing…" : "Pair"}
                  </button>
                )}
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
          placeholder="Text to send to the selected device…"
          rows={3}
        />
        <button onClick={shareClipboard} disabled={!selected || !clipboard}>
          Send clipboard
        </button>
      </section>

      {prompts[0] && (
        <PairingDialog prompt={prompts[0]} onAnswer={(accept) => answerPrompt(prompts[0], accept)} />
      )}
      <Toasts {...toasts} />
    </main>
  );
}

function applyTransferEvent(list: TransferView[], e: TransferEvent, peerName: string): TransferView[] {
  const existing = list.find((t) => t.id === e.id);
  const base: TransferView = existing ?? {
    id: e.id,
    filename: e.filename,
    direction: e.direction,
    peerName,
    bytes: 0,
    total: 0,
    speedBps: 0,
    etaSecs: null,
    status: "active",
  };
  let next: TransferView;
  switch (e.type) {
    case "progress":
      next = {
        ...base,
        bytes: e.bytesSent,
        total: e.totalBytes,
        speedBps: e.speedBps,
        etaSecs: e.etaSecs,
      };
      break;
    case "transferComplete":
      next = { ...base, bytes: e.totalBytes, total: e.totalBytes, status: "done", path: e.path };
      break;
    case "transferError":
      next = { ...base, status: "failed", error: e.error };
      break;
  }
  // Newest first.
  return existing ? list.map((t) => (t.id === e.id ? next : t)) : [next, ...list];
}
