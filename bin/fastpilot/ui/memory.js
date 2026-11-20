// fastpilot ui: Memory (D6, v0.4). What fastpilot has learned about each test, in words, and the
// two things a person can do: Set aside / Bring back, and Forget. Uses h(), icon(), api(), toast()
// from app.js. Values from the server are always set as text, never as HTML. Everything is inside
// one function, so no name here meets one in app.js; only PAGES.memory is shared.
(() => {
"use strict";

let memData = null; // { tests: [...], mode }
let memBusy = false;
let memPick = null; // id of the test in the side panel

const STATUS = {
  reliable: ["Reliable", "ok"],
  unstable: ["Unstable", "warn"],
  failing: ["Failing", "bad"],
  set_aside: ["Set aside", "grey"],
  new: ["New", "new"],
};
const PLATFORM_NAME = { web: "Web", android: "Android emulator", ios: "iPhone simulator" };

async function loadMemory() {
  memBusy = true;
  try {
    memData = await api("/api/memory");
  } catch (e) {
    memData = { tests: [], mode: "on", error: e.message };
  }
  memBusy = false;
  if ((location.hash || "").startsWith("#/memory")) pageMemory();
}

function whenText(ms) {
  if (!ms) return "never";
  const d = new Date(ms);
  const time = d.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
  const day = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(new Date()) - day(d)) / 86400000);
  if (diff === 0) return `Today, ${time}`;
  if (diff === 1) return `Yesterday, ${time}`;
  return `${d.toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" })}, ${time}`;
}

async function act(action, t) {
  try {
    const r = await api("/api/memory", { action, id: t.id });
    memData = { tests: r.tests, mode: memData.mode };
    toast(r.ok, true);
  } catch (e) {
    toast("Could not change Memory: " + e.message);
  }
  pageMemory();
}

// The one action that fits the row, with its consequence as the button's description.
function rowAction(t) {
  if (t.status === "set_aside") return h("button", { class: "btn sm", onclick: () => act("bring_back", t), title: "Its failures count again." }, "Bring back");
  if (t.status === "unstable" || t.status === "failing") return h("button", { class: "btn sm", onclick: () => act("set_aside", t), title: "It still runs, but a failure does not turn the run red." }, "Set aside");
  if (t.steps > 0 || t.recent_runs > 0) return h("button", { class: "btn sm", onclick: () => act("forget", t), title: "The next run works the steps out again." }, "Forget");
  return null;
}

function panel(t) {
  const [label, cls] = STATUS[t.status] || [t.status, ""];
  const facts = [
    ["Status", h("span", { class: "mem-st " + cls }, label)],
    ["Last worked", whenText(t.last_worked_ms)],
    t.path.length ? ["Path it remembers", t.path.join(" → ")] : null,
    t.recent_runs ? ["Recent runs", t.recent_fails ? `Failed ${t.recent_fails} of the last ${t.recent_runs}` : `Passed the last ${t.recent_runs}`] : null,
    t.set_aside_since_ms ? ["Set aside", whenText(t.set_aside_since_ms) + (t.set_aside_auto ? " (by fastpilot)" : "")] : null,
  ].filter(Boolean);
  const explain = {
    unstable: "Unstable means it passes sometimes and fails sometimes without any change. Worth fixing, but not always a bug in your app.",
    failing: "Failing means its last runs all failed. Open the latest report in History to see where.",
    set_aside: "Set aside: it still runs and is shown, but a failure does not turn the run red.",
    new: "Nothing is remembered until the test passes once. After that, runs get faster.",
  }[t.status];
  const buttons = [];
  if (t.status === "set_aside") buttons.push(h("button", { class: "btn sm p", onclick: () => act("bring_back", t) }, "Bring back"));
  else if (t.status !== "new") buttons.push(h("button", { class: "btn sm p", onclick: () => act("set_aside", t) }, "Set aside"));
  if (t.steps > 0 || t.recent_runs > 0) buttons.push(h("button", { class: "btn sm", onclick: () => act("forget", t) }, "Forget this"));
  return h("aside", { class: "mem-panel", "aria-label": "Selected test" },
    h("div", { class: "mem-k" }, "Selected test"),
    h("h2", { tabindex: "-1" }, t.title),
    t.orphan ? h("p", { class: "s muted" }, "Its test file is gone; only what fastpilot remembers is left.") : null,
    h("dl", { class: "mem-facts" }, facts.map(([k, v]) => [h("dt", {}, k), h("dd", {}, v)])),
    explain ? h("p", { class: "mem-note" }, explain) : null,
    buttons.length ? h("div", { class: "btns tight mem-btns" }, buttons) : null,
    h("p", { class: "s muted" }, h("b", {}, "Forget this: "), "the next run works the steps out again and may take a little longer. Your test itself is not changed."),
    h("p", { class: "s muted" }, h("b", {}, "Set aside: "), "the test still runs and is shown, but a failure does not turn the run red."));
}

function pageMemory() {
  $top().replaceChildren(icon("memory"), h("h1", { tabindex: "-1" }, "Memory"), h("span", { class: "muted hist-sub" }, "What fastpilot has learned about your tests"),
    h("div", { class: "hist-tabs", role: "tablist" },
      h("button", { class: "hist-tab on", role: "tab", "aria-selected": "true" }, "Tests"),
      h("button", { class: "hist-tab", role: "tab", "aria-selected": "false", disabled: true, title: "Coming soon" }, "Project notes · soon")));
  if (!memData) {
    $body().replaceChildren(h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" }), h("b", {}, "Reading memory…")));
    if (!memBusy) loadMemory();
    return;
  }
  const tests = memData.tests || [];
  const notes = [];
  if (memData.error) notes.push(h("div", { class: "err", role: "alert" }, "Could not read Memory: " + memData.error));
  if (memData.mode === "off") notes.push(h("div", { class: "mem-note" }, "Memory is off for this project (FASTPILOT_MEMORY=off): nothing is remembered or used."));
  if (memData.mode === "read-only") notes.push(h("div", { class: "mem-note" }, "Memory is read-only here (FASTPILOT_MEMORY=readonly): runs use it but do not change it."));
  const remembered = tests.some((t) => t.status !== "new");
  if (!tests.length || !remembered) {
    $body().replaceChildren(...notes, h("div", { class: "grp" }, h("div", { class: "empty" }, icon("memory", "big"), h("b", {}, "Nothing remembered yet"),
      h("span", {}, "After a test passes once, fastpilot remembers how, and runs get faster."))));
    return;
  }
  const pick = tests.find((t) => t.id === memPick) || tests.find((t) => t.status === "unstable" || t.status === "failing") || tests[0];
  memPick = pick.id;
  const table = h("div", { class: "grp" }, h("div", { class: "tbl-wrap" }, h("table", { class: "tbl mem-tbl" },
    h("thead", {}, h("tr", {}, ["Test", "What fastpilot knows", "Status", ""].map((t) => h("th", {}, t)))),
    h("tbody", {}, tests.map((t) => {
      const [label, cls] = STATUS[t.status] || [t.status, ""];
      return h("tr", { class: t.id === pick.id ? "on" : "" },
        h("td", {}, h("button", { class: "row-name mem-name", "aria-pressed": String(t.id === pick.id), onclick: () => { memPick = t.id; pageMemory(); document.querySelector(".mem-panel h2")?.focus(); } }, t.title),
          h("div", { class: "s" }, PLATFORM_NAME[t.platform] || t.platform || "Test file not found")),
        h("td", { class: t.status === "new" ? "muted" : "" }, t.summary),
        h("td", {}, h("span", { class: "mem-st " + cls }, label)),
        h("td", { class: "acts" }, rowAction(t)));
    })))));
  $body().replaceChildren(...notes, h("div", { class: "mem-wrap" }, table, panel(pick)));
}

PAGES.memory = () => {
  memData = null;
  pageMemory();
};
})();
