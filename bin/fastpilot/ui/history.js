// fastpilot ui: History (D7, v0.4). Past runs and Chat conversations of this project; delete one,
// several, or all; auto-delete after N days. Uses h(), icon(), api(), toast() from app.js.
// Values from the server are always set as text, never as HTML. Everything is inside one function,
// so no name here meets one in app.js; only PAGES.history is shared.
(() => {
"use strict";

let histData = null; // { runs, chats, keep_days }
let histBusy = false;
let histTab = "runs"; // runs | chats
let histFilter = "all"; // all | attention | passed
let histQuery = "";
const histSel = { runs: new Set(), chats: new Set() };
let histOpenChat = null; // { id, title, messages }

async function loadHistory() {
  histBusy = true;
  try {
    histData = await api("/api/history");
    // Rows deleted elsewhere (auto-delete) leave the selection.
    const ids = { runs: new Set(histData.runs.map((r) => r.id)), chats: new Set(histData.chats.map((c) => c.id)) };
    for (const k of ["runs", "chats"]) for (const id of [...histSel[k]]) if (!ids[k].has(id)) histSel[k].delete(id);
  } catch (e) {
    histData = { runs: [], chats: [], keep_days: null, error: e.message };
  }
  histBusy = false;
  if ((location.hash || "").startsWith("#/history")) pageHistory();
}

function when(ms) {
  if (!ms) return "—";
  const d = new Date(ms);
  const time = d.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
  const day = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(new Date()) - day(d)) / 86400000);
  if (diff === 0) return `Today, ${time}`;
  if (diff === 1) return `Yesterday, ${time}`;
  return `${d.toLocaleDateString("en-GB", { day: "numeric", month: "short" })}, ${time}`;
}

const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
const PLATFORM = { web: "Web", android: "Android emulator", ios: "iPhone simulator" };

function result(r) {
  if (r.failed > 0) return h("span", { class: "hist-res bad" }, `${r.failed} ${r.failed === 1 ? "needs" : "need"} attention`);
  if (r.not_run > 0) return h("span", { class: "hist-res warn" }, `${r.not_run} could not run`);
  return h("span", { class: "hist-res ok" }, "All passed");
}

const attention = (r) => r.failed > 0 || r.not_run > 0;

function visibleRuns() {
  const q = histQuery.trim().toLowerCase();
  return histData.runs.filter((r) =>
    (histFilter === "all" || (histFilter === "attention" ? attention(r) : !attention(r))) &&
    (!q || r.title.toLowerCase().includes(q)));
}

function visibleChats() {
  const q = histQuery.trim().toLowerCase();
  return histData.chats.filter((c) => !q || c.title.toLowerCase().includes(q));
}

// A confirmation with the safe button focused first. Resolves true only on the red button.
function histConfirm(title, text, okLabel) {
  return UI.confirmDialog({ title, text, verb: okLabel, keep: "Keep them" });
}

async function histChange(body, okText) {
  try {
    const r = await api("/api/history", body);
    const d = r.deleted || { runs: 0, chats: 0 };
    const parts = [];
    if (d.runs) parts.push(plural(d.runs, "run", "runs"));
    if (d.chats) parts.push(plural(d.chats, "conversation", "conversations"));
    toast(r.note || (parts.length ? `Deleted ${parts.join(" and ")}.` : okText || "Saved."), !r.note);
    if (histOpenChat && !histData.chats.some((c) => c.id === histOpenChat.id)) histOpenChat = null;
    histSel.runs.clear();
    histSel.chats.clear();
  } catch (e) {
    toast("Could not change History: " + e.message);
  }
  await loadHistory();
}

const LOST_RUNS = "The reports, screenshots and videos of these runs are deleted from this computer.";
const LOST_CHATS = "These conversations are deleted from this computer.";
const STAYS = "This cannot be undone. Your tests and schedules stay.";

async function deleteSelected() {
  const kind = histTab;
  const ids = [...histSel[kind]];
  if (!ids.length) return;
  const what = kind === "runs" ? plural(ids.length, "run", "runs") : plural(ids.length, "conversation", "conversations");
  const ok = await histConfirm(`Delete ${what}?`, `${kind === "runs" ? LOST_RUNS : LOST_CHATS} ${STAYS}`, `Delete ${what}`);
  if (ok) await histChange(kind === "runs" ? { action: "delete", runs: ids, chats: [] } : { action: "delete", runs: [], chats: ids });
}

async function deleteAll() {
  const n = histData.runs.length, m = histData.chats.length;
  if (!n && !m) return;
  const what = [n ? plural(n, "run", "runs") : null, m ? plural(m, "conversation", "conversations") : null].filter(Boolean).join(" and ");
  const ok = await histConfirm(`Delete all ${what}?`, `${n ? LOST_RUNS + " " : ""}${m ? LOST_CHATS + " " : ""}${STAYS}`, "Delete all");
  if (ok) await histChange({ action: "delete_all" });
}

async function setKeepDays(value) {
  const days = value === "" ? null : Number(value);
  if (days) {
    const cutoff = Date.now() - days * 86400000;
    const n = histData.runs.filter((r) => r.finished_ms && r.finished_ms < cutoff).length;
    const m = histData.chats.filter((c) => c.updated_ms && c.updated_ms < cutoff).length;
    if (n || m) {
      const what = [n ? plural(n, "run", "runs") : null, m ? plural(m, "conversation", "conversations") : null].filter(Boolean).join(" and ");
      const ok = await histConfirm(`Delete ${what} older than ${days} days now?`,
        `Auto-delete after ${days} days removes them right away, and later ones as they get older. ${STAYS}`, `Turn on and delete ${what}`);
      if (!ok) { pageHistory(); return; }
    }
  }
  await histChange({ action: "keep_days", days }, days ? `Runs and conversations are deleted after ${days} days.` : "Auto-delete is off: everything is kept.");
}

async function openChat(id) {
  try {
    histOpenChat = await api("/api/history/chat?id=" + encodeURIComponent(id));
  } catch (e) {
    toast("Could not open the conversation: " + e.message);
    histOpenChat = null;
  }
  pageHistory();
  document.querySelector(".hist-chat h2")?.focus();
}

function selBox(kind, id, label) {
  return h("input", { type: "checkbox", "aria-label": label, checked: histSel[kind].has(id),
    onchange: (e) => { if (e.target.checked) histSel[kind].add(id); else histSel[kind].delete(id); pageHistory(); } });
}

function allBox(kind, rows) {
  const ids = rows.map((r) => r.id);
  const on = ids.filter((id) => histSel[kind].has(id)).length;
  const box = h("input", { type: "checkbox", "aria-label": "Select all shown", checked: ids.length > 0 && on === ids.length,
    onchange: (e) => { ids.forEach((id) => (e.target.checked ? histSel[kind].add(id) : histSel[kind].delete(id))); pageHistory(); } });
  box.indeterminate = on > 0 && on < ids.length;
  return box;
}

function runsTable() {
  const rows = visibleRuns();
  if (!rows.length) return h("div", { class: "empty small" }, histData.runs.length ? "No run matches." : "No runs yet. They show up here after your first test.");
  return h("div", { class: "tbl-wrap" }, h("table", { class: "tbl hist-tbl" },
    h("thead", {}, h("tr", {}, h("th", { class: "cb" }, allBox("runs", rows)), ["Run", "Result", "When", ""].map((t) => h("th", {}, t)))),
    h("tbody", {}, rows.map((r) => {
      const where = (r.platforms || []).map((p) => PLATFORM[p] || p).join(" + ");
      return h("tr", { class: histSel.runs.has(r.id) ? "sel" : "" },
        h("td", { class: "cb" }, selBox("runs", r.id, `Select ${r.title}`)),
        h("td", {}, h("div", { class: "row-name" }, r.title), h("div", { class: "s" }, [plural(r.tests, "test", "tests"), where].filter(Boolean).join(" · "))),
        h("td", {}, result(r)),
        h("td", { class: "next" }, when(r.finished_ms)),
        h("td", { class: "acts" }, r.has_report
          ? h("a", { class: "btn sm", href: "/api/history/report?id=" + encodeURIComponent(r.id), target: "_blank", rel: "noopener" }, "Open")
          : h("span", { class: "s muted" }, "No report")));
    }))));
}

function chatsTable() {
  const rows = visibleChats();
  if (!rows.length) return h("div", { class: "empty small" }, histData.chats.length ? "No conversation matches." : "No conversations yet. Chat keeps each one here.");
  return h("div", { class: "tbl-wrap" }, h("table", { class: "tbl hist-tbl" },
    h("thead", {}, h("tr", {}, h("th", { class: "cb" }, allBox("chats", rows)), ["Conversation", "Last message", ""].map((t) => h("th", {}, t)))),
    h("tbody", {}, rows.map((c) => h("tr", { class: histSel.chats.has(c.id) ? "sel" : "" },
      h("td", { class: "cb" }, selBox("chats", c.id, `Select ${c.title}`)),
      h("td", {}, h("div", { class: "row-name" }, c.title), h("div", { class: "s" }, plural(c.messages, "message", "messages"))),
      h("td", { class: "next" }, when(c.updated_ms)),
      h("td", { class: "acts" }, h("button", { class: "btn sm", onclick: () => openChat(c.id) }, "Open")))))));
}

function chatPanel() {
  const c = histOpenChat;
  const msgs = Array.isArray(c.messages) ? c.messages : [];
  return h("section", { class: "grp hist-chat", "aria-label": "Saved conversation" },
    h("div", { class: "gh" }, h("h2", { tabindex: "-1" }, c.title || "Conversation"),
      h("button", { class: "btn sm right", onclick: () => { histOpenChat = null; pageHistory(); } }, "Close")),
    h("div", { class: "hist-msgs" }, msgs.map((m) => h("div", { class: "hist-msg " + (m.role === "user" ? "me" : "bot") },
      h("div", { class: "who" }, m.role === "user" ? "You" : "fastpilot"),
      h("div", { class: "txt" }, m.text || (m.error ? "" : "…")),
      m.error ? h("div", { class: "txt bad" }, m.error) : null,
      (m.tools || []).length ? h("div", { class: "s muted" }, "Used: " + m.tools.join(", ")) : null))),
    h("div", { class: "s muted hist-ro" }, "Read only. To continue, start a new conversation in Chat."));
}

function pageHistory() {
  const tab = (key, label, n) => h("button", { class: "hist-tab" + (histTab === key ? " on" : ""), role: "tab", "aria-selected": String(histTab === key),
    onclick: () => { histTab = key; histQuery = ""; pageHistory(); } }, label, h("span", { class: "cnt" }, String(n)));
  const keep = histData ? histData.keep_days : null;
  const options = [["", "Auto-delete: off"], ["7", "Auto-delete: after 7 days"], ["30", "Auto-delete: after 30 days"], ["90", "Auto-delete: after 90 days"]];
  if (keep && !options.some(([v]) => Number(v) === keep)) options.push([String(keep), `Auto-delete: after ${keep} days`]);
  const select = h("select", { class: "sel", "aria-label": "Auto-delete", disabled: !histData, onchange: (e) => setKeepDays(e.target.value) },
    options.map(([v, t]) => h("option", { value: v, selected: String(keep || "") === v }, t)));
  const total = histData ? histData.runs.length + histData.chats.length : 0;
  $top().replaceChildren(icon("history"), h("h1", { tabindex: "-1" }, "History"), h("span", { class: "muted hist-sub" }, "Past runs and conversations"),
    h("div", { class: "hist-tabs", role: "tablist" }, tab("runs", "Runs", histData ? histData.runs.length : 0), tab("chats", "Conversations", histData ? histData.chats.length : 0)),
    h("div", { class: "right hist-top" }, select, h("button", { class: "btn sm danger", disabled: !total, onclick: deleteAll }, "Delete all…")));
  if (!histData) {
    $body().replaceChildren(h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" }), h("b", {}, "Reading history…")));
    if (!histBusy) loadHistory();
    return;
  }
  if (!total) {
    $body().replaceChildren(...(histData.error ? [h("div", { class: "err", role: "alert" }, "Could not read History: " + histData.error)] : []),
      h("div", { class: "grp" }, h("div", { class: "empty" }, icon("history", "big"), h("b", {}, "No history yet"),
        h("span", {}, "Runs and conversations show up here after your first test."))));
    return;
  }
  const kind = histTab;
  const n = histSel[kind].size;
  const filters = kind === "runs"
    ? h("div", { class: "hist-filters", role: "group", "aria-label": "Show" }, [["all", "All"], ["attention", "Needs attention"], ["passed", "Passed"]].map(([k, t]) =>
      h("button", { class: "hist-f" + (histFilter === k ? " on" : ""), "aria-pressed": String(histFilter === k), onclick: () => { histFilter = k; pageHistory(); } }, t)))
    : null;
  const search = h("input", { class: "hist-q", type: "search", placeholder: "Search history", "aria-label": "Search history", value: histQuery,
    oninput: (e) => { histQuery = e.target.value; const at = e.target.selectionStart; pageHistory(); const q = document.querySelector(".hist-q"); q?.focus(); q?.setSelectionRange(at, at); } });
  const bar = h("div", { class: "hist-bar" }, filters, search, h("span", { class: "sp" }),
    n ? h("span", { class: "muted" }, `${n} selected`) : null,
    n ? h("button", { class: "btn sm danger", onclick: deleteSelected }, `Delete ${kind === "runs" ? plural(n, "run", "runs") : plural(n, "conversation", "conversations")}…`) : null);
  $body().replaceChildren(...(histData.error ? [h("div", { class: "err", role: "alert" }, "Could not read History: " + histData.error)] : []),
    bar, h("div", { class: "grp" }, kind === "runs" ? runsTable() : chatsTable()),
    ...(kind === "chats" && histOpenChat ? [chatPanel()] : []));
}

PAGES.history = () => {
  histData = null;
  pageHistory();
};
})();
