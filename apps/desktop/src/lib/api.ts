// Typed wrappers around the Tauri commands in src-tauri/src/commands.rs.
// Keep the types here in sync with the Rust structs (serde renames to camelCase).
import { invoke } from "@tauri-apps/api/core";

export type OsType = "Android" | "Linux" | "Windows" | "MacOS";

export interface DeviceInfo {
  id: string;
  name: string;
  os: OsType;
  /** Short hash of the device's identity key, for users to compare. */
  fingerprint: string;
}

export interface Peer {
  device: DeviceInfo;
  addresses: string[];
  port: number;
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
