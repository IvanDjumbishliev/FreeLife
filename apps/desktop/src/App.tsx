import { useCallback, useEffect, useRef, useState } from "react";
import {
  discoverDevices,
  getClipboardSettings,
  getDeviceInfo,
  getNotificationSettings,
  listPairedDevices,
  onHandoffEvent,
  onClipboardEvent,
  onNotificationEvent,
  onPairingDismiss,
  onPairingPrompt,
  onTransferEvent,
  pairDevice,
  respondPairing,
  sendFile,
  sendNotificationReply,
  dismissNotification,
  setClipboardSettings,
  setNotificationSettings,
  unpairDevice,
  type ClipboardSettings,
  type MirroredNotification,
  type HandoffEntry,
  type NotificationSettings,
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
import { ClipboardSettingsPanel } from "./components/ClipboardSettings";
import { ClipboardToast } from "./components/ClipboardToast";
import { WebcamCard } from "./components/WebcamCard";
import { Notifications } from "./components/Notifications";
import { HandoffHistory } from "./components/HandoffHistory";

export default function App() {
  const [self, setSelf] = useState<DeviceInfo | null>(null);
  const [paired, setPaired] = useState<PairedDevice[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [nearby, setNearby] = useState<NearbyDevice[]>([]);
  const [scanning, setScanning] = useState(false);
  const [pairing, setPairing] = useState<string | null>(null);
  const [prompts, setPrompts] = useState<PairingPrompt[]>([]);
  const [transfers, setTransfers] = useState<TransferView[]>([]);
  const [clipboardSettings, setClipboardSettingsState] = useState<ClipboardSettings | null>(null);
  const [clipboardNotice, setClipboardNotice] = useState<string | null>(null);
  const [notificationSettings, setNotificationSettingsState] =
    useState<NotificationSettings | null>(null);
  const [notifications, setNotifications] = useState<MirroredNotification[]>(
    loadNotifications,
  );
  const [notificationApps, setNotificationApps] = useState(loadNotificationApps);
  const [handoffs, setHandoffs] = useState<HandoffEntry[]>(loadHandoffs);
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
    getClipboardSettings().then(setClipboardSettingsState).catch((e) => push(String(e), "error"));
    getNotificationSettings()
      .then((settings) => {
        setNotificationSettingsState(settings);
        const blocked = new Set(settings.blockedApps);
        setNotifications((list) => list.filter((n) => !blocked.has(n.appPackage)));
      })
      .catch((e) => push(String(e), "error"));
    refreshPaired();
    // Connection state changes aren't evented yet, so poll gently.
    const timer = setInterval(refreshPaired, 5000);
    return () => clearInterval(timer);
  }, [refreshPaired, push]);

  useEffect(() => {
    try {
      localStorage.setItem("freelife.notifications", JSON.stringify(notifications));
    } catch (error) {
      console.warn("Could not persist mirrored notifications", error);
    }
  }, [notifications]);

  useEffect(() => {
    try {
      localStorage.setItem("freelife.notification-apps", JSON.stringify(notificationApps));
    } catch (error) {
      console.warn("Could not persist notification app list", error);
    }
  }, [notificationApps]);

  useEffect(() => {
    try {
      localStorage.setItem("freelife.handoffs", JSON.stringify(handoffs));
    } catch (error) {
      console.warn("Could not persist handoff history", error);
    }
  }, [handoffs]);

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
    onClipboardEvent((e) => {
      if (e.type === "received") {
        setClipboardNotice(`Clipboard synced from ${e.deviceName}`);
      } else if (e.type === "skipped") {
        push(`Clipboard not synced: ${e.reason}`, "info");
      } else if (e.type === "settingsChanged") {
        setClipboardSettingsState(e.settings);
      }
    }),
  );
  useTauriEvent(() =>
    onNotificationEvent((event) => {
      if (event.appPackage) {
        setNotificationApps((apps) => {
          const next = new Map(apps.map((app) => [app.appPackage, app]));
          next.set(event.appPackage, {
            appPackage: event.appPackage,
            appName: event.appName,
          });
          return [...next.values()];
        });
      }
      setNotifications((list) => {
        const key = notificationKey(event);
        const withoutCurrent = list.filter((item) => notificationKey(item) !== key);
        return event.removed || event.blocked
          ? withoutCurrent
          : [event, ...withoutCurrent].slice(0, 50);
      });
    }),
  );
  useTauriEvent(() =>
    onHandoffEvent((event) => setHandoffs((list) => [event, ...list].slice(0, 20))),
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

  function loadNotificationApps(): Array<{ appPackage: string; appName: string }> {
    try {
      const stored: unknown = JSON.parse(
        localStorage.getItem("freelife.notification-apps") ?? "[]",
      );
      if (!Array.isArray(stored)) return [];
      return stored.filter(
        (item): item is { appPackage: string; appName: string } =>
          typeof item === "object" &&
          item !== null &&
          typeof (item as Record<string, unknown>).appPackage === "string" &&
          typeof (item as Record<string, unknown>).appName === "string",
      );
    } catch (error) {
      console.warn("Could not load notification app list", error);
      return [];
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

  async function changeClipboardSettings(next: ClipboardSettings) {
    setClipboardSettingsState(next);
    try {
      await setClipboardSettings(next);
    } catch (e) {
      push(String(e), "error");
    }
  }

  async function changeNotificationSettings(next: NotificationSettings) {
    setNotificationSettingsState(next);
    const blocked = new Set(next.blockedApps);
    setNotifications((list) => list.filter((notification) => !blocked.has(notification.appPackage)));
    try {
      await setNotificationSettings(next);
    } catch (e) {
      push(String(e), "error");
    }
  }

  async function replyToNotification(
    notification: MirroredNotification,
    text: string,
  ): Promise<boolean> {
    if (!notification.replyToken) return false;
    try {
      await sendNotificationReply(notification.deviceId, notification.replyToken, text);
      push(`Reply sent to ${notification.appName}`, "success");
      return true;
    } catch (e) {
      push(`Reply failed: ${e}`, "error");
      return false;
    }
  }

  async function dismissMirroredNotification(notification: MirroredNotification) {
    try {
      await dismissNotification(notification.deviceId, notification.id);
      setNotifications((list) =>
        list.filter((item) => notificationKey(item) !== notificationKey(notification)),
      );
    } catch (e) {
      push(`Could not dismiss notification: ${e}`, "error");
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
        <WebcamCard
          phone={paired.find((p) => p.connected && p.device.os === "Android") ?? null}
          onError={(message) => push(message, "error")}
        />
      </section>

      <section>
        <h2>Transfers</h2>
        <Transfers transfers={transfers} />
      </section>

      <Notifications
        notifications={notifications}
        apps={notificationApps}
        settings={notificationSettings}
        onSettingsChange={changeNotificationSettings}
        onReply={replyToNotification}
        onDismiss={dismissMirroredNotification}
      />

      <HandoffHistory entries={handoffs} />

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
        {clipboardSettings && (
          <ClipboardSettingsPanel
            settings={clipboardSettings}
            paired={paired}
            onChange={changeClipboardSettings}
          />
        )}
        <p className="muted small">
          Windows and macOS sync as you copy. Linux desktops with Wayland
          sync when the compositor allows clipboard access.
        </p>
      </section>

      {prompts[0] && (
        <PairingDialog prompt={prompts[0]} onAnswer={(accept) => answerPrompt(prompts[0], accept)} />
      )}
      <Toasts {...toasts} />
      <ClipboardToast message={clipboardNotice} onDone={() => setClipboardNotice(null)} />
    </main>
  );
}

function loadHandoffs(): HandoffEntry[] {
  try {
    const stored: unknown = JSON.parse(localStorage.getItem("freelife.handoffs") ?? "[]");
    if (!Array.isArray(stored)) return [];
    return stored
      .filter((item): item is HandoffEntry => {
        if (typeof item !== "object" || item === null) return false;
        const value = item as Record<string, unknown>;
        return (
          typeof value.url === "string" &&
          typeof value.timestamp === "number" &&
          typeof value.sourceDeviceId === "string" &&
          typeof value.sourceDeviceName === "string"
        );
      })
      .slice(0, 20);
  } catch (error) {
    console.warn("Could not load handoff history", error);
    return [];
  }
}

function notificationKey(notification: MirroredNotification): string {
  return `${notification.deviceId}:${notification.id}`;
}

function loadNotifications(): MirroredNotification[] {
  try {
    const stored: unknown = JSON.parse(
      localStorage.getItem("freelife.notifications") ?? "[]",
    );
    if (!Array.isArray(stored)) return [];
    return stored
      .filter((item): item is MirroredNotification => {
        if (typeof item !== "object" || item === null) return false;
        const value = item as Record<string, unknown>;
        return (
          typeof value.id === "string" &&
          typeof value.deviceId === "string" &&
          typeof value.appName === "string" &&
          typeof value.title === "string" &&
          typeof value.timestamp === "number" &&
          typeof value.removed === "boolean" &&
          typeof value.blocked === "boolean"
        );
      })
      .slice(0, 50);
  } catch (error) {
    console.warn("Could not load mirrored notifications", error);
    return [];
  }
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
