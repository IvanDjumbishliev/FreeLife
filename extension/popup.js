const api = globalThis.browser ?? globalThis.chrome;

const $ = (id) => document.getElementById(id);

async function activeTab() {
  const [tab] = await api.tabs.query({ active: true, currentWindow: true });
  return tab;
}

async function renderHistory() {
  const { history = [] } = await api.storage.local.get("history");
  $("history").replaceChildren(
    ...history.map(({ url }) => {
      const li = document.createElement("li");
      li.textContent = url;
      li.title = url;
      return li;
    }),
  );
}

async function init() {
  const tab = await activeTab();
  $("current").textContent = tab?.url ?? "No active tab";
  $("send").disabled = !tab?.url;

  $("send").addEventListener("click", async () => {
    const res = await api.runtime.sendMessage({ type: "handoff", url: tab.url });
    $("status").textContent = res?.ok ? "Sent (stub — not delivered yet)" : "Failed";
    renderHistory();
  });

  renderHistory();
}

init();
