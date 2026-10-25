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

export interface ClipboardSettings {
  enabled: boolean;
  trustedOnly: boolean;
  trustedDevices: string[];
  blockSecrets: boolean;
  syncImages: boolean;
}

export type ContentKind = "text" | "url" | "image";

/** Summaries only: clipboard content itself never reaches the UI. */
export type ClipboardEvent =
  | { type: "sent"; hash: string; kind: ContentKind; devices: number }
  | {
      type: "received";
      from: string;
      deviceName: string;
      kind: ContentKind;
      hash: string;
    }
  | { type: "skipped"; hash: string; kind: ContentKind; reason: string }
  | { type: "settingsChanged"; settings: ClipboardSettings };

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

export const getClipboardSettings = () =>
  invoke<ClipboardSettings>("get_clipboard_settings");

export const setClipboardSettings = (settings: ClipboardSettings) =>
  invoke<void>("set_clipboard_settings", { settings });

export const onTransferEvent = (cb: (e: TransferEvent) => void): Promise<UnlistenFn> =>
  listen<TransferEvent>("transfer-event", (e) => cb(e.payload));

export const onPairingPrompt = (cb: (p: PairingPrompt) => void): Promise<UnlistenFn> =>
  listen<PairingPrompt>("pairing-prompt", (e) => cb(e.payload));

export const onPairingDismiss = (cb: (requestId: number) => void): Promise<UnlistenFn> =>
  listen<{ requestId: number }>("pairing-dismiss", (e) => cb(e.payload.requestId));

export const onClipboardEvent = (cb: (e: ClipboardEvent) => void): Promise<UnlistenFn> =>
  listen<ClipboardEvent>("clipboard-event", (e) => cb(e.payload));

export type CameraEvent =
  | { type: "started"; width: number; height: number }
  | {
      type: "stats";
      width: number;
      height: number;
      fps: number;
      latencyMs: number | null;
      framesTotal: number;
      decodeErrors: number;
      sink: string;
    }
  | { type: "preview"; pngBase64: string }
  | { type: "ended" }
  | { type: "sinkError"; message: string };

export const phoneWebcamStart = (deviceId: string) =>
  invoke<void>("phone_webcam_start", { deviceId });

export const phoneWebcamStop = () => invoke<void>("phone_webcam_stop");

export const phoneWebcamStatus = () => invoke<string | null>("phone_webcam_status");

export const onCameraEvent = (cb: (e: CameraEvent) => void): Promise<UnlistenFn> =>
  listen<CameraEvent>("camera-event", (e) => cb(e.payload));
