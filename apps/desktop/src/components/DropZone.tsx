import { useEffect, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";

interface Props {
  /** Name of the device files will go to, or null if none is selected. */
  target: string | null;
  onDrop: (paths: string[]) => void;
}

/**
 * Highlights while files are dragged over the window and hands dropped
 * paths to `onDrop`. Tauri intercepts OS file drops (HTML5 drop events
 * don't carry real paths), so this listens on the webview instead.
 */
export function DropZone({ target, onDrop }: Props) {
  const [hovering, setHovering] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        if (payload.type === "enter" || payload.type === "over") {
          setHovering(true);
        } else if (payload.type === "drop") {
          setHovering(false);
          if (payload.paths.length > 0) onDrop(payload.paths);
        } else {
          setHovering(false);
        }
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [onDrop]);

  const classes = ["dropzone", hovering && "dropzone-active", !target && "dropzone-disabled"]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={classes}>
      {target ? (
        <>
          <strong>Drop files here</strong>
          <span className="muted">to send them to {target}</span>
        </>
      ) : (
        <span className="muted">Select a paired device to send files</span>
      )}
    </div>
  );
}
