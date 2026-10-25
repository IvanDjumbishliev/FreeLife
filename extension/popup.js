const api = globalThis.browser ?? globalThis.chrome;

const current = document.getElementById("current");
const devices = document.getElementById("devices");
const status = document.getElementById("status");

async function init() {
  try {
    const [tab] = await api.tabs.query({ active: true, currentWindow: true });
    const url = tab?.url;
    current.textContent = url ?? "No active tab";
    if (!url || !/^https?:\/\//i.test(url)) {
      status.textContent = "Only http and https tabs can be handed off.";
      return;
    }

    const response = await api.runtime.sendMessage({ type: "listDevices" });
    for (const device of response.devices ?? []) {
      const item = document.createElement("li");
      const button = document.createElement("button");
      button.textContent = `Send to ${device.name}`;
      button.addEventListener("click", async () => {
        button.disabled = true;
        status.textContent = `Sending to ${device.name}…`;
        try {
          await api.runtime.sendMessage({
            type: "handoff",
            deviceId: device.id,
            url,
          });
          status.textContent = `Sent to ${device.name}`;
        } catch (error) {
          status.textContent = error.message;
          button.disabled = false;
        }
      });
      item.append(button);
      devices.append(item);
    }
    if (!devices.childElementCount) status.textContent = "No paired devices found.";
  } catch (error) {
    status.textContent = error.message;
  }
}

init();
