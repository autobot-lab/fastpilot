// fastpilot ui: the page frame (v0.7.1 FC4, HANDOFF-fc4-fc5 "Layout"). Left, the agents sidebar like
// OpenBot: "+ New agent", every agent (only the open one shows its chats: the 5 newest, "Show all",
// "+ New chat", a spinner on a chat that is answering), and Live, Device and Settings at the foot.
// Above the page, the agent's name and app with its tabs (Chat · Skills · Schedules · History ·
// Memory · Map), or the page's own title. Under about 900 px the sidebar is a drawer behind a Menu
// button. Uses h(), icon(), api() and toast() from app.js. Server text is always set as text.
(() => {
"use strict";

const TABS = [["chat", "Chat"], ["skills", "Skills"], ["schedules", "Schedules"], ["history", "History"], ["memory", "Memory"], ["map", "Map"]];
const SHOWN = 5;
const KEY = "fastpilot.agent";
let data = null; // /api/agents
let chats = { agent: null, list: [] }; // /api/chats of the open agent
let showAll = false;
let timer = null;
let lastFocus = null;
let switched = null; // the agent and app the app in use was last moved to

const agentId = () => localStorage.getItem(KEY) || "default";
function setAgent(id) {
  if (id && id !== "new" && id !== agentId()) {
    localStorage.setItem(KEY, id);
    showAll = false;
  }
}
const byId = (id) => ((data && data.agents) || []).find((x) => x.agent.id === id);
const busyAny = () => ((data && data.agents) || []).some((x) => x.busy) || chats.list.some((c) => c.busy);

async function load() {
  try {
    const id = agentId();
    const [a, c] = await Promise.all([api("/api/agents"), api("/api/chats?agent=" + encodeURIComponent(id))]);
    data = a;
    chats = { agent: id, list: c.chats || [] };
    if (!byId(id) && id !== "default") { localStorage.removeItem(KEY); return load(); }
    // FC5: an agent tests its own app: opening it makes that app the one in use (refused, and
    // left as it is, while a chat is answering or a map is being made).
    const mine = byId(id);
    if (mine && mine.app_id && mine.app_id !== a.current_app && switched !== `${id}:${mine.app_id}`) {
      switched = `${id}:${mine.app_id}`;
      try { await api("/api/apps", { action: "use", id: mine.app_id }); data.current_app = mine.app_id; } catch (_) { /* stays */ }
    }
  } catch (_) { /* the sidebar keeps what it had */ }
  draw();
  clearTimeout(timer);
  timer = setTimeout(load, busyAny() ? 2000 : 15000);
}

// The chat open now: `#/agents/<agent>/chat/<id>` ("new" for a new chat).
function route() {
  const p = (location.hash || "").split("/");
  return { page: p[1] || "", agent: p[1] === "agents" ? p[2] : null, tab: p[1] === "agents" ? p[3] || "chat" : p[1], chat: p[1] === "agents" && p[3] === "chat" ? p[4] || null : null };
}

function avatar(x) {
  return h("span", { class: "avt " + ((x.agent.id.length % 2) ? "g" : "b"), "aria-hidden": "true" }, x.initials);
}

function chatRow(x, c, current) {
  const on = c.id === current;
  const title = c.title || "New chat";
  const del = h("button", { type: "button", class: "del", "aria-label": `Remove chat “${title}”`, title: "Remove chat", onclick: (e) => { e.preventDefault(); e.stopPropagation(); removeChat(x.agent.id, c, title); } }, icon("close", "sm"));
  return h("li", { class: "crow-li" },
    h("a", { class: "crow" + (on ? " on" : ""), href: `#/agents/${x.agent.id}/chat/${c.id}`, "aria-current": on ? "page" : null, title },
      h("span", { class: "t" }, title),
      c.busy ? h("span", { class: "spin", role: "img", "aria-label": c.waiting !== null && c.waiting !== undefined ? "Waiting" : "Working" }) : null),
    del);
}

function agentBlock(x, r) {
  const open = x.agent.id === agentId();
  const head = h("a", { class: "aghd" + (open ? " on" : ""), href: `#/agents/${x.agent.id}/chat`, "aria-current": open && r.page === "agents" && r.tab !== "chat" ? "page" : null },
    avatar(x), h("span", { class: "agtx" }, h("b", {}, x.agent.name), h("small", {}, x.app_line || "No app yet")),
    !open && x.busy ? h("span", { class: "spin", role: "img", "aria-label": "A chat is working" }) : null,
    open ? null : icon("chev", "sm"));
  if (!open) return h("li", { class: "agent" }, head);
  const list = chats.agent === x.agent.id ? chats.list : [];
  const shown = showAll ? list : list.slice(0, SHOWN);
  const current = r.agent === x.agent.id ? r.chat : null;
  return h("li", { class: "agent open" }, head,
    h("ul", { class: "chats", "aria-label": `Chats of ${x.agent.name}` },
      shown.map((c) => chatRow(x, c, current)),
      list.length > SHOWN && !showAll ? h("li", {}, h("button", { type: "button", class: "crow mute", onclick: () => { showAll = true; draw(); } }, `Show all (${list.length})`)) : null,
      h("li", {}, h("a", { class: "crow newc" + (current === "new" ? " on" : ""), href: `#/agents/${x.agent.id}/chat/new`, "aria-current": current === "new" ? "page" : null }, "+ New chat"))));
}

function draw() {
  const side = document.getElementById("side");
  if (!side) return;
  const r = route();
  const foot = [["live", "Live", "live", "#/live"], ["device", "Device", "device", "#/device"], ["settings", "Settings", "settings", "#/settings/limits"]];
  const keepToast = side.querySelector(".stoast");
  side.replaceChildren(
    h("div", { class: "brand" }, h("span", { class: "lg2", "aria-hidden": "true" }, "f"), "fastpilot"),
    h("a", { class: "newagent", href: "#/agents/new" }, icon("plus", "sm"), "New agent"),
    h("nav", { class: "agents", "aria-label": "Agents" }, h("ul", {}, ((data && data.agents) || []).map((x) => agentBlock(x, r)))),
    h("nav", { class: "sidefoot", "aria-label": "Pages" }, foot.map(([k, t, ic, href]) => {
      const on = r.page === k || (k === "settings" && r.page === "integrations");
      return h("a", { href, class: on ? "on" : "", "aria-current": on ? "page" : null }, icon(ic, "sm"), t);
    })),
    ...(keepToast ? [keepToast] : []));
  header();
}

// The title row above the page: an agent's name, app and tabs; or the page's own name.
function header() {
  const hdr = document.getElementById("hdr");
  if (!hdr) return;
  const r = route();
  const agentPage = r.page === "agents" || ["schedules", "history", "memory", "map"].includes(r.page);
  document.body.classList.toggle("in-agent", agentPage);
  if (!agentPage) {
    hdr.hidden = true;
    hdr.replaceChildren();
    return;
  }
  hdr.hidden = false;
  if (r.agent === "new") {
    hdr.replaceChildren(h("div", { class: "nm" }, h("h1", {}, "New agent"), h("span", { class: "sub" }, "No app yet")));
    return;
  }
  const x = byId(r.agent || agentId());
  if (!x) { hdr.replaceChildren(); return; }
  const tab = r.page === "agents" ? r.tab : r.page;
  // Before the first saved test there is only Chat (HANDOFF item 5): no tab row.
  const tabs = data.saved_tests > 0 || tab !== "chat"
    ? h("nav", { class: "tabs2", "aria-label": `${x.agent.name}: sections` }, TABS.map(([k, t]) => {
      const href = k === "chat" || k === "skills" ? `#/agents/${x.agent.id}/${k}` : `#/${k}`;
      const on = k === tab;
      return h("a", { href, class: on ? "on" : "", "aria-current": on ? "page" : null }, t);
    }))
    : null;
  hdr.replaceChildren(h("div", { class: "nm" }, h("a", { class: "agname", href: `#/agents/${x.agent.id}/about`, title: "About this agent" }, h("h1", {}, x.agent.name)), h("span", { class: "sub" }, x.app_line || "No app yet")), ...(tabs ? [tabs] : []));
}

// ---- chats: remove with Undo (FC6), from the trash button ----
let toastTimer = null;
async function removeChat(agent, c, title) {
  let r;
  try { r = await api("/api/chat", { agent, chat: c.id, action: "remove" }); } catch (e) { toast(e.message); return; }
  const cur = route();
  if (cur.chat === c.id) location.hash = `#/agents/${agent}/chat/new`;
  await load();
  sideToast(r.message === "Stopped and removed" ? "Stopped and removed" : "Chat removed", r.undo, title);
}

function sideToast(text, undo, title) {
  const side = document.getElementById("side");
  if (!side) return;
  side.querySelector(".stoast")?.remove();
  clearTimeout(toastTimer);
  const close = () => { box.remove(); };
  const box = h("div", { class: "stoast", role: "status" },
    h("div", { class: "stx" }, h("b", {}, text), h("small", {}, `“${title}” · Kept 7 days`)),
    undo ? h("button", { type: "button", class: "u", onclick: async () => {
      try { await api("/api/undo", { id: undo }); close(); await load(); toast(`Restored chat “${title}”`, true); } catch (e) { toast(e.message); }
    } }, "Undo") : null,
    h("button", { type: "button", class: "x", "aria-label": "Dismiss", onclick: close }, icon("close", "sm")));
  side.append(box);
  toastTimer = setTimeout(close, 20000);
}

// ---- narrow window: the sidebar as a drawer ----
function openDrawer(e) {
  lastFocus = e && e.currentTarget;
  document.body.classList.add("drawer");
  document.getElementById("menubtn")?.setAttribute("aria-expanded", "true");
  const side = document.getElementById("side");
  side.setAttribute("role", "dialog");
  side.setAttribute("aria-modal", "true");
  side.setAttribute("aria-label", "Menu");
  document.getElementById("scrim").hidden = false;
  (side.querySelector(".crow.on, .aghd.on, a") || side).focus();
}
function closeDrawer() {
  if (!document.body.classList.contains("drawer")) return;
  document.body.classList.remove("drawer");
  document.getElementById("menubtn")?.setAttribute("aria-expanded", "false");
  const side = document.getElementById("side");
  side.removeAttribute("role");
  side.removeAttribute("aria-modal");
  side.removeAttribute("aria-label");
  document.getElementById("scrim").hidden = true;
  const back = lastFocus && lastFocus.isConnected ? lastFocus : document.getElementById("menubtn");
  if (back) back.focus();
}
document.addEventListener("keydown", (e) => {
  if (!document.body.classList.contains("drawer")) return;
  if (e.key === "Escape") { e.preventDefault(); closeDrawer(); return; }
  if (e.key !== "Tab") return;
  // Focus stays in the drawer while it is open.
  const items = [...document.getElementById("side").querySelectorAll("a[href], button:not([disabled])")];
  if (!items.length) return;
  const first = items[0], last = items[items.length - 1];
  if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
  else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
});
window.addEventListener("DOMContentLoaded", () => {
  document.getElementById("scrim")?.addEventListener("click", closeDrawer);
  document.getElementById("menubtn")?.addEventListener("click", openDrawer);
  // Choosing a chat or a page closes the drawer.
  document.getElementById("side")?.addEventListener("click", (e) => { if (e.target.closest("a[href]")) setTimeout(closeDrawer); });
});

window.Shell = {
  // Called by the router on every page: which agent is open, then the frame is drawn again.
  frame() {
    const r = route();
    if (r.page === "agents" && r.agent) setAgent(r.agent);
    if (!data || chats.agent !== agentId()) load(); else draw();
  },
  refresh: load,
  agent: agentId,
  setAgent,
  agentData: (id) => byId(id || agentId()),
  data: () => data,
  // A chat's own list entry (newest first): the chat to open for `#/agents/<id>/chat`.
  newestChat: () => (chats.agent === agentId() && chats.list.length ? chats.list[0].id : null),
  closeDrawer,
};
})();
