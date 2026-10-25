import { useEffect } from "react";

interface Props {
  message: string | null;
  onDone: () => void;
}

/** Subtle notice that disappears after 2 seconds. */
export function ClipboardToast({ message, onDone }: Props) {
  useEffect(() => {
    if (!message) return;
    const t = setTimeout(onDone, 2000);
    return () => clearTimeout(t);
  }, [message, onDone]);

  if (!message) return null;
  return (
    <div className="clipboard-toast" role="status" aria-live="polite">
      {message}
    </div>
  );
}
