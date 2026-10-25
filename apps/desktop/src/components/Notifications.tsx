import { useState, type FormEvent } from "react";
import type { MirroredNotification, NotificationSettings } from "../lib/api";

interface Props {
  notifications: MirroredNotification[];
  apps: Array<{ appPackage: string; appName: string }>;
  settings: NotificationSettings | null;
  onSettingsChange: (settings: NotificationSettings) => void;
  onReply: (notification: MirroredNotification, text: string) => Promise<boolean>;
  onDismiss: (notification: MirroredNotification) => Promise<void>;
}

export function Notifications({
  notifications,
  apps,
  settings,
  onSettingsChange,
  onReply,
  onDismiss,
}: Props) {
  const groups = new Map<string, MirroredNotification[]>();
  for (const notification of notifications) {
    const group = groups.get(notification.appName) ?? [];
    group.push(notification);
    groups.set(notification.appName, group);
  }

  return (
    <>
      <section>
        <h2>Notifications</h2>
        {settings && (
          <label className="setting">
            <input
              type="checkbox"
              checked={settings.enabled}
              onChange={(e) =>
                onSettingsChange({ ...settings, enabled: e.target.checked })
              }
            />
            <span>Enable mirrored notifications</span>
          </label>
        )}
        {notifications.length === 0 ? (
          <p className="muted">No mirrored notifications yet.</p>
        ) : (
          [...groups].map(([appName, entries]) => (
            <div className="notification-group" key={appName}>
              <h3>{appName}</h3>
              <ul className="notifications">
                {entries.map((notification) => (
                  <NotificationRow
                    key={`${notification.deviceId}:${notification.id}`}
                    notification={notification}
                    onReply={onReply}
                    onDismiss={onDismiss}
                  />
                ))}
              </ul>
            </div>
          ))
        )}
      </section>

      <section>
        <h2>Notification settings</h2>
        {!settings || apps.length === 0 ? (
          <p className="muted">App filters appear when notifications arrive.</p>
        ) : (
          <div className="settings">
            {apps.map(({ appPackage, appName }) => {
              const blocked = settings.blockedApps.includes(appPackage);
              return (
                <label className="setting" key={appPackage}>
                  <input
                    type="checkbox"
                    checked={blocked}
                    onChange={(e) => {
                      const blockedApps = new Set(settings.blockedApps);
                      if (e.target.checked) blockedApps.add(appPackage);
                      else blockedApps.delete(appPackage);
                      onSettingsChange({
                        ...settings,
                        blockedApps: [...blockedApps],
                      });
                    }}
                  />
                  <span>Block notifications from {appName}</span>
                </label>
              );
            })}
          </div>
        )}
      </section>
    </>
  );
}

interface RowProps {
  notification: MirroredNotification;
  onReply: Props["onReply"];
  onDismiss: Props["onDismiss"];
}

function NotificationRow({ notification, onReply, onDismiss }: RowProps) {
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!notification.replyToken || !text.trim() || sending) return;
    setSending(true);
    try {
      if (await onReply(notification, text)) setText("");
    } finally {
      setSending(false);
    }
  }

  return (
    <li className="notification">
      {notification.iconData && (
        <img className="notification-icon" src={notification.iconData} alt="" />
      )}
      <div className="notification-content">
        <div className="notification-heading">
          <strong>{notification.title || notification.appName}</strong>
          <span className="muted small">
            {new Date(notification.timestamp).toLocaleTimeString()}
          </span>
        </div>
        {notification.body && <p>{notification.body}</p>}
        <div className="notification-actions">
          {notification.hasReplyAction && notification.replyToken && (
            <form className="notification-reply" onSubmit={submit}>
              <input
                aria-label={`Reply to ${notification.appName}`}
                value={text}
                onChange={(event) => setText(event.target.value)}
                placeholder="Quick reply"
                maxLength={4096}
              />
              <button type="submit" disabled={sending || !text.trim()}>
                Reply
              </button>
            </form>
          )}
          <button className="link" onClick={() => void onDismiss(notification)}>
            Dismiss
          </button>
        </div>
      </div>
    </li>
  );
}
