// Typed wrappers around the Tauri commands in src-tauri/src/commands.rs.
// Keep the types here in sync with the Rust structs (serde renames to camelCase).
import { invoke } from "@tauri-apps/api/core";

export type Platform =
  | "android"
  | "ios"
  | "linux"
  | "windows"
  | "macos"
  | "browser"
  | "unknown";

export interface DeviceInfo {
  id: string;
  name: string;
  platform: Platform;
  port: number;
}

export interface Peer {
  info: DeviceInfo;
  addresses: string[];
}

export interface TransferReceipt {
  transferId: string;
  chunks: number;
}

export const getDeviceInfo = () => invoke<DeviceInfo>("get_device_info");

export const discoverDevices = (timeoutMs = 2000) =>
  invoke<Peer[]>("discover_devices", { timeoutMs });

export const sendClipboard = (peerId: string, text: string) =>
  invoke<void>("send_clipboard", { peerId, text });

export const sendFile = (peerId: string, path: string) =>
  invoke<TransferReceipt>("send_file", { peerId, path });
