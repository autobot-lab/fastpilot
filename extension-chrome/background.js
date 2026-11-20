// fastpilot attach: bridges one tab's chrome.debugger session to fastpilot's localhost relay
// (ws://127.0.0.1:<port>/extension). Frames are plain CDP JSON; child sessions (OOPIFs, workers)
// are addressed with sessionId (Chrome 125+). Click the toolbar button to attach / detach.

let state = null; // { tabId, ws, ping }

async function settings() {
  const { port = 9230, token = "" } = await chrome.storage.local.get(["port", "token"]);
  return { port, token };
}

// The relay's token changes with every fastpilot session. fastpilot hands it over through native
// messaging (host dev.fastpilot.relay, registered when a session waits for the extension): web
// pages cannot reach it. The token in the options is only the fallback (FASTPILOT_RELAY_TOKEN).
async function relayToken(port, fallback) {
  try {
    const r = await chrome.runtime.sendNativeMessage("dev.fastpilot.relay", { port });
    if (r && r.token) return r.token;
    if (r && r.error) console.warn("fastpilot:", r.error);
  } catch (e) {
    console.warn("fastpilot: native host unavailable, using the token from the options:", e && e.message);
  }
  return fallback;
}

function badge(text, color) {
  chrome.action.setBadgeText({ text });
  if (color) chrome.action.setBadgeBackgroundColor({ color });
}

async function detach(reason) {
  if (!state) return;
  const { tabId, ws, ping } = state;
  state = null;
  clearInterval(ping);
  try { ws.close(); } catch (_) {}
  try { await chrome.debugger.detach({ tabId }); } catch (_) {}
  badge("", null);
  console.log("fastpilot: detached", reason || "");
}

async function attach(tab) {
  const { port, token: fallback } = await settings();
  const token = await relayToken(port, fallback);
  // No token, no connection: whatever listens on the port could otherwise drive this tab. The
  // debugger is not attached either.
  if (!token) {
    badge("KEY", "#cf222e");
    console.error(`fastpilot: no relay token for port ${port}: start a fastpilot session with endpoint: extension first (it hands the token over), or set the token from FASTPILOT_RELAY_TOKEN in the options`);
    return;
  }
  await chrome.debugger.attach({ tabId: tab.id }, "1.3");
  const ws = new WebSocket(`ws://127.0.0.1:${port}/extension?token=${encodeURIComponent(token)}`);
  // An active WebSocket keeps the service worker alive; the ping keeps traffic flowing.
  const ping = setInterval(() => { if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ id: -1, method: "Runtime.evaluate", params: { expression: "1" } })); }, 20000);
  state = { tabId: tab.id, ws, ping };
  badge("…", "#9a6700");

  ws.onopen = () => badge("ON", "#1a7f37");
  ws.onclose = () => detach("relay closed");
  ws.onerror = () => detach("relay unreachable (is fastpilot waiting? session start mode: attach, endpoint: extension)");
  ws.onmessage = async (ev) => {
    let msg;
    try { msg = JSON.parse(ev.data); } catch (_) { return; }
    if (msg.id === undefined || !msg.method) return;
    const target = msg.sessionId ? { tabId: tab.id, sessionId: msg.sessionId } : { tabId: tab.id };
    try {
      const result = await chrome.debugger.sendCommand(target, msg.method, msg.params || {});
      if (msg.id >= 0) ws.send(JSON.stringify({ id: msg.id, sessionId: msg.sessionId, result: result || {} }));
    } catch (e) {
      if (msg.id >= 0) ws.send(JSON.stringify({ id: msg.id, sessionId: msg.sessionId, error: { code: -32000, message: String(e && e.message || e) } }));
    }
  };
}

chrome.debugger.onEvent.addListener((source, method, params) => {
  if (!state || source.tabId !== state.tabId || state.ws.readyState !== WebSocket.OPEN) return;
  const frame = { method, params };
  if (source.sessionId) frame.sessionId = source.sessionId;
  state.ws.send(JSON.stringify(frame));
});

chrome.debugger.onDetach.addListener((source, reason) => {
  if (state && source.tabId === state.tabId) detach(reason);
});

chrome.action.onClicked.addListener(async (tab) => {
  if (state) return detach("toggled off");
  try { await attach(tab); } catch (e) { badge("ERR", "#cf222e"); console.error("fastpilot:", e); }
});
