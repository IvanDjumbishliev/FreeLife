const api = globalThis.browser ?? globalThis.chrome;
const NATIVE_HOST = "dev.freelife.handoff";

api.runtime.onMessage.addListener((message) => {
  if (message?.type === "listDevices") {
    return nativeRequest({ op: "getDevices" });
  }
  if (message?.type === "handoff") {
    return nativeRequest({
      op: "handoff",
      deviceId: message.deviceId,
      url: message.url,
    });
  }
  return undefined;
});

function nativeRequest(request) {
  return new Promise((resolve, reject) => {
    let settled = false;
    const port = api.runtime.connectNative(NATIVE_HOST);
    const timer = setTimeout(() => finish(new Error("FreeLife desktop did not respond")), 20000);
    const finish = (error, response) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      port.disconnect();
      if (error) reject(error);
      else resolve(response);
    };
    port.onMessage.addListener((response) => {
      if (response?.ok) finish(null, response);
      else finish(new Error(response?.error ?? "FreeLife desktop rejected the request"));
    });
    port.onDisconnect.addListener(() => {
      if (!settled) {
        finish(new Error(api.runtime.lastError?.message ?? "Native host disconnected"));
      }
    });
    port.postMessage(request);
  });
}
