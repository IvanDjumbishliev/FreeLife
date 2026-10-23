// FreeLife Handoff background script.
// Runs as a service worker in Chrome and as an event page in Firefox.

const api = globalThis.browser ?? globalThis.chrome;

const MENU_PAGE = "freelife-send-page";
const MENU_LINK = "freelife-send-link";

api.runtime.onInstalled.addListener(() => {
  api.contextMenus.create({
    id: MENU_PAGE,
    title: "Send page to FreeLife device",
    contexts: ["page"],
  });
  api.contextMenus.create({
    id: MENU_LINK,
    title: "Send link to FreeLife device",
    contexts: ["link"],
  });
});

api.contextMenus.onClicked.addListener((info, tab) => {
  const url = info.menuItemId === MENU_LINK ? info.linkUrl : tab?.url;
  if (url) handoff(url);
});

api.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg?.type === "handoff" && typeof msg.url === "string") {
    handoff(msg.url).then(sendResponse);
    return true; // keep the channel open for the async response
  }
  return false;
});

/**
 * STUB: hand a URL off to another device.
 *
 * TODO: forward to the local FreeLife desktop app (which relays it to the
 * paired device as a `Message::OpenUrl`). For now we just record it.
 */
async function handoff(url) {
  const entry = { url, at: Date.now() };
  const { history = [] } = await api.storage.local.get("history");
  await api.storage.local.set({ history: [entry, ...history].slice(0, 20) });
  console.log("[FreeLife] handoff (stub):", url);
  return { ok: true, stub: true };
}
