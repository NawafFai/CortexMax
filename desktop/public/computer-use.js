// The bar Cortex Max floats above every window while an agent uses the
// computer. The desktop hands it the agents at the controls — first through
// `window.__CORTEX_COMPUTER_USE__`, then by calling `cortexComputerUse` —
// and Stop ends that agent's run.
(() => {
  const bar = document.getElementById("bar");
  let agents = Array.isArray(window.__CORTEX_COMPUTER_USE__)
    ? window.__CORTEX_COMPUTER_USE__
    : [];
  const stopping = new Map();

  function ago(atMs) {
    const seconds = Math.max(0, Math.round((Date.now() - atMs) / 1000));
    return seconds < 2 ? "الحين" : `قبل ${seconds} ث`;
  }

  function stop(agent) {
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    if (!invoke || stopping.has(agent.pubkey)) return;
    stopping.set(agent.pubkey, "يوقف…");
    render();
    invoke("stop_computer_use_agent", {
      pubkey: agent.pubkey,
      relayUrl: agent.relayUrl ?? null,
    }).then(
      () => {
        stopping.set(agent.pubkey, "توقف");
        render();
      },
      () => {
        stopping.delete(agent.pubkey);
        render();
      },
    );
  }

  function row(agent) {
    const line = document.createElement("div");
    line.className = agent.controls ? "row" : "row looking";

    const dot = document.createElement("span");
    dot.className = "dot";

    const text = document.createElement("div");
    text.className = "text";
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = agent.name;
    const detail = document.createElement("span");
    detail.className = "detail";
    const where = agent.app ? ` في ${agent.app}` : "";
    detail.textContent = ` — ${agent.action}${where} · ${ago(agent.atMs)}`;
    text.append(
      name,
      document.createTextNode(agent.controls ? " يتحكم بجهازك" : " يشوف شاشتك"),
      detail,
    );
    text.title = `${agent.name}: ${agent.action}${where}`;

    const button = document.createElement("button");
    const state = stopping.get(agent.pubkey);
    button.textContent = state ?? "إيقاف";
    button.disabled = state !== undefined;
    button.title = `أوقف ${agent.name}`;
    button.addEventListener("click", () => stop(agent));

    line.append(dot, text, button);
    return line;
  }

  function render() {
    bar.replaceChildren(...agents.map(row));
  }

  window.cortexComputerUse = (next) => {
    agents = Array.isArray(next) ? next : [];
    for (const pubkey of stopping.keys()) {
      if (!agents.some((agent) => agent.pubkey === pubkey))
        stopping.delete(pubkey);
    }
    render();
  };

  render();
  setInterval(render, 1000);
})();
