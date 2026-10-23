import { useEffect } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";

/** Subscribe to a Tauri event for the component's lifetime. */
export function useTauriEvent(subscribe: () => Promise<UnlistenFn>, deps: unknown[] = []) {
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    subscribe().then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, deps);
}
