// Typed wrappers around the Tauri commands in src-tauri/src/commands.rs and
// the events in src-tauri/src/events.rs. Keep in sync with the Rust structs.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type OsType = "Android" | "Linux" | "Windows" | "MacOS";

export interface DeviceInfo {
  id: string;
  name: string;
  os: OsType;
  /** Short hash of the device's identity key, for users to compare. */
  fingerprint: string;
}

export interface NearbyDevice {
  device: DeviceInfo;
  addresses: string[];
  paired: boolean;
}

export interface PairedDevice {
  device: DeviceInfo;
  connected: boolean;
}

export type Direction = "send" | "receive";

export interface TransferProgress {
  id: string;
  peer: string;
  direction: Direction;
  filename: string;
  bytesSent: number;
  totalBytes: number;
  speedBps: number;
  etaSecs: number | null;
}

export type TransferEvent =
  | ({ type: "progress" } & TransferProgress)
  | {
      type: "transferComplete";
      id: string;
      peer: string;
      direction: Direction;
      filename: string;
      totalBytes: number;
      path: string | null;
    }
  | {
      type: "transferError";
      id: string;
      peer: string;
      direction: Direction;
      filename: string;
      error: string;
    };

export interface PairingPrompt {
  requestId: number;
  device: DeviceInfo;
  code: string;
}

export interface ClipboardReceived {
  from: string;
  text: string;
}

export const getDeviceInfo = () => invoke<DeviceInfo>("get_device_info");

export const discoverDevices = (timeoutMs = 2000) =>
  invoke<NearbyDevice[]>("discover_devices", { timeoutMs });

export const listPairedDevices = () => invoke<PairedDevice[]>("list_paired_devices");

export const pairDevice = (deviceId: string) =>
  invoke<DeviceInfo>("pair_device", { deviceId });

export const respondPairing = (requestId: number, accept: boolean) =>
  invoke<boolean>("respond_pairing", { requestId, accept });

export const unpairDevice = (deviceId: string) =>
  invoke<boolean>("unpair_device", { deviceId });

export const sendFile = (deviceId: string, filePath: string) =>
  invoke<void>("send_file", { deviceId, filePath });

export const sendClipboard = (deviceId: string, text: string) =>
  invoke<void>("send_clipboard", { deviceId, text });

export const onTransferEvent = (cb: (e: TransferEvent) => void): Promise<UnlistenFn> =>
  listen<TransferEvent>("transfer-event", (e) => cb(e.payload));

export const onPairingPrompt = (cb: (p: PairingPrompt) => void): Promise<UnlistenFn> =>
  listen<PairingPrompt>("pairing-prompt", (e) => cb(e.payload));

export const onPairingDismiss = (cb: (requestId: number) => void): Promise<UnlistenFn> =>
  listen<{ requestId: number }>("pairing-dismiss", (e) => cb(e.payload.requestId));

export const onClipboardReceived = (cb: (c: ClipboardReceived) => void): Promise<UnlistenFn> =>
  listen<ClipboardReceived>("clipboard-received", (e) => cb(e.payload));
