import type { ClipboardSettings as Settings, PairedDevice } from "../lib/api";

interface Props {
  settings: Settings;
  paired: PairedDevice[];
  onChange: (next: Settings) => void;
}

export function ClipboardSettingsPanel({ settings, paired, onChange }: Props) {
  const set = (patch: Partial<Settings>) => onChange({ ...settings, ...patch });
  const trusted = new Set(settings.trustedDevices);

  return (
    <div className="settings">
      <label className="setting">
        <input
          type="checkbox"
          checked={settings.enabled}
          onChange={(e) => set({ enabled: e.target.checked })}
        />
        <span>
          <strong>Clipboard Sync: {settings.enabled ? "On" : "Off"}</strong>
          <span className="muted small">
            Copies on this computer appear on paired devices, and theirs here.
          </span>
        </span>
      </label>

      <label className="setting">
        <input
          type="checkbox"
          checked={settings.trustedOnly}
          disabled={!settings.enabled}
          onChange={(e) => set({ trustedOnly: e.target.checked })}
        />
        <span>
          Trusted devices only
          <span className="muted small">
            Sync only with devices checked below.
          </span>
        </span>
      </label>
      {settings.trustedOnly && settings.enabled && (
        <ul className="trusted-list">
          {paired.map(({ device }) => (
            <li key={device.id}>
              <label>
                <input
                  type="checkbox"
                  checked={trusted.has(device.id)}
                  onChange={(e) => {
                    const next = new Set(trusted);
                    if (e.target.checked) next.add(device.id);
                    else next.delete(device.id);
                    set({ trustedDevices: [...next] });
                  }}
                />{" "}
                {device.name}
              </label>
            </li>
          ))}
        </ul>
      )}

      <label className="setting">
        <input
          type="checkbox"
          checked={settings.blockSecrets}
          onChange={(e) => set({ blockSecrets: e.target.checked })}
        />
        <span>
          Never sync passwords and keys
          <span className="muted small">
            Skips content that looks like a password, private key or API token.
          </span>
        </span>
      </label>

      <label className="setting">
        <input
          type="checkbox"
          checked={settings.syncImages}
          onChange={(e) => set({ syncImages: e.target.checked })}
        />
        <span>
          Sync images
          <span className="muted small">Screenshots and other copied images.</span>
        </span>
      </label>
    </div>
  );
}
