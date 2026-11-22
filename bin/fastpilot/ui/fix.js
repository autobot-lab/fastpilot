// fastpilot ui: the Fix buttons of the Device page (D2, v0.4). fastpilot installs Chrome or sets up
// its own Android test phone only after a person clicks here and confirms; Chat cannot start a
// fix. Uses h(), icon(), api(), toast() from app.js. Values from the server are always set as
// text, never as HTML. Everything is inside one function; only PAGES.deviceFix is shared.
(() => {
"use strict";

let fixData = null; // { offers, job }
let fixBusy = false;
let fixTimer = null;
let fixDetails = false;
let lastStart = null; // the body of the last start, for Try again

async function loadFix() {
  if (fixBusy) return;
  fixBusy = true;
  try {
    fixData = await api("/api/fix");
  } catch (e) {
    fixData = { offers: [], job: null, error: e.message };
  }
  fixBusy = false;
  clearTimeout(fixTimer);
  if ((location.hash || "").startsWith("#/device")) {
    redraw();
    if (fixData.job && fixData.job.state === "running") fixTimer = setTimeout(loadFix, 1000);
  }
}

function redraw() {
  const old = document.getElementById("fix-section");
  if (old) old.replaceWith(section());
}

// The server answers a refused fix with {error}; api() keeps only the text.
async function fixPost(body) {
  const r = await fetch("/api/fix", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  const text = await r.text();
  let v = null;
  try { v = JSON.parse(text); } catch (_) { v = null; }
  if (!r.ok) throw new Error((v && v.error) || text);
  return v;
}

const ASK = { chrome: ["Install Chrome?", "Install"], android: ["Set up an Android emulator?", "Set up"], ios: ["Set up iPhone testing?", "Set up"] };

// `ask`: [title, button] of the dialog (a start of an installed emulator has its own).
function confirmFix(offer, ask) {
  const [title, yes] = ask || ASK[offer.area] || ["Go ahead?", "OK"];
  return new Promise((resolve) => {
    const done = (v) => { dlg.close(); dlg.remove(); resolve(v); };
    const cancel = h("button", { class: "btn", onclick: () => done(false) }, "Cancel");
    const dlg = h("dialog", { class: "hist-dlg", "aria-labelledby": "fix-dlg-t" },
      h("h2", { id: "fix-dlg-t" }, title),
      h("p", {}, offer.confirm),
      (offer.steps || []).length ? h("ol", { class: "fix-plan" }, offer.steps.map((s) => h("li", {}, s))) : null,
      h("div", { class: "btns tight" }, cancel, h("button", { class: "btn p", onclick: () => done(true) }, yes)));
    dlg.addEventListener("cancel", (e) => { e.preventDefault(); done(false); });
    document.body.append(dlg);
    dlg.showModal();
    cancel.focus();
  });
}

async function startFix(offer, choice) {
  const ask = choice ? [`Start ${choice.name}?`, "Start"] : null;
  if (!(await confirmFix(choice ? { area: offer.area, confirm: choice.confirm, steps: [] } : offer, ask))) return;
  lastStart = { action: "start", area: offer.area, confirmed: true, ...(choice ? { avd: choice.id } : {}) };
  await run(lastStart);
}

async function run(body) {
  try {
    const r = await fixPost(body);
    fixData = { ...fixData, job: r.job };
    fixDetails = false;
  } catch (e) {
    toast("Could not start: " + e.message);
  }
  loadFix();
}

// Which simulator tests use: a setting, saved at once (nothing is installed or started).
async function pickSimulator(choice) {
  try {
    const r = await fixPost({ action: "pick-simulator", udid: choice.id });
    toast(r.ok, true);
  } catch (e) { toast(e.message); }
  loadFix();
}

async function stopFix() {
  try { await fixPost({ action: "stop" }); } catch (e) { toast(e.message); }
  loadFix();
}

const MARK = { done: "✓", running: "▶", failed: "✗", stopped: "■", waiting: "○" };

function jobPanel(job) {
  const n = job.steps.length;
  const done = job.steps.filter((s) => s.state === "done").length;
  const current = job.steps.findIndex((s) => s.state === "running");
  const fill = h("i", {});
  fill.style.width = Math.round((done / Math.max(1, n)) * 100) + "%";
  const running = job.state === "running";
  const head = running ? job.doing || job.title : job.state === "done" ? "Done" : job.state === "stopped" ? "Stopped" : "Could not finish";
  return h("div", { class: "fix-job", role: "status", "aria-live": "polite" },
    h("div", { class: "fix-head" }, h("b", {}, head), running && current >= 0 ? h("span", { class: "pill" }, `Step ${current + 1} of ${n}`) : null),
    h("div", { class: "fix-bar" }, fill),
    h("ul", { class: "fix-steps" }, job.steps.map((s) => h("li", { class: s.state }, h("span", { "aria-hidden": "true" }, MARK[s.state] || "○"), " ", s.label, s.state === "running" ? "…" : ""))),
    job.message ? h("p", { class: job.state === "failed" ? "note-err" : "s" }, job.message) : null,
    h("div", { class: "btns tight fix-acts" },
      running ? h("button", { class: "btn sm", onclick: stopFix }, "Stop") : null,
      !running && job.state !== "done" ? h("button", { class: "btn sm", onclick: () => { if (lastStart && lastStart.area === job.area) run(lastStart); else loadFix(); } }, "Try again") : null,
      h("button", { class: "btn sm", "aria-expanded": String(fixDetails), onclick: () => { fixDetails = !fixDetails; redraw(); } }, fixDetails ? "Hide details" : "Show details")),
    fixDetails ? h("pre", { class: "fix-log", tabindex: "0", "aria-label": "Details" }, job.log.length ? job.log.join("\n") : "Nothing yet.") : null);
}

// S10 + DC4 (v0.7): one line per kind (website, Android, iPhone) with the device picked for the
// person; every device of that kind is under Change. Buttons name the kind, never an ID.
async function startLine(l) {
  const ok = await UI.confirmDialog({ title: l.button + "?", text: l.confirm || "", verb: l.button, keep: "Cancel", danger: false });
  if (!ok) return;
  if (l.start.action === "boot-simulator") {
    try { toast((await fixPost({ action: "boot-simulator", confirmed: true })).ok, true); } catch (e) { toast("Could not start: " + e.message); }
    loadFix();
    return;
  }
  lastStart = { ...l.start, confirmed: true };
  await run(lastStart);
}

async function useDevice(l, c) {
  if (c.act === "start") {
    const ok = await UI.confirmDialog({ title: `Start ${c.name}?`, text: c.confirm || "", verb: "Start", keep: "Cancel", danger: false });
    if (!ok) return;
    lastStart = { action: "start", area: "android", avd: c.id, confirmed: true };
    await run(lastStart);
  } else if (l.kind === "android") {
    try { toast((await fixPost({ action: "pick-android", serial: c.id })).ok, true); } catch (e) { toast(e.message); }
    loadFix();
  } else {
    await pickSimulator(c);
  }
}

async function answerAutostart(on) {
  try { toast((await fixPost({ action: "autostart", on })).ok, true); } catch (e) { toast(e.message); }
  loadFix();
}

function lineRows(l, busy) {
  const state = l.ready ? "ok" : l.button ? "warn" : "fail";
  const row = h("div", { class: "row dev-line", "data-kind": l.kind },
    h("div", {},
      h("div", { class: "t" }, h("span", { class: "dot " + state, "aria-hidden": "true" }), l.name),
      h("div", { class: "s" }, l.detail)),
    l.button ? h("button", { class: "btn sm p", disabled: busy, onclick: () => startLine(l) }, l.button) : null);
  const items = (l.choices || []).map((c) => h("li", { "data-id": c.id },
    h("span", {}, h("b", {}, c.name), c.note ? h("span", { class: "s" }, ` · ${c.note}`) : null),
    c.button ? h("button", { class: "btn sm", disabled: busy, onclick: () => useDevice(l, c) }, c.button) : null));
  // fastpilot's own test phone as a second choice (DV1), when the Android offer has it.
  const add = l.kind === "android" ? (fixData.offers || []).find((o) => o.area === "android" && o.button && o.secondary) : null;
  if (!items.length && !add) return [row];
  const label = l.kind === "ios" ? "iPhone simulators on this computer" : "Android devices on this computer";
  return [row, UI.moreOptions({ label: "Change" },
    items.length ? h("ul", { class: "fix-choices", "aria-label": label }, items) : null,
    add ? h("div", { class: "fix-more" }, h("button", { class: "btn sm", disabled: busy, onclick: () => startFix(add) }, add.button), h("span", { class: "s" }, " (downloads about 1.5 GB)")) : null)];
}

function autostartRow() {
  if (fixData.ask_autostart) {
    return h("div", { class: "row dev-auto", role: "group", "aria-labelledby": "dev-auto-q" },
      h("div", {}, h("div", { class: "t", id: "dev-auto-q" }, "Start it automatically when a test needs it?"),
        h("div", { class: "s" }, "When a test needs the Android emulator or the iPhone simulator and it is not running. Asked once.")),
      h("div", { class: "btns tight" },
        h("button", { class: "btn sm p", onclick: () => answerAutostart(true) }, "Yes"),
        h("button", { class: "btn sm", onclick: () => answerAutostart(false) }, "No")));
  }
  if (fixData.autostart === true || fixData.autostart === false) {
    const on = fixData.autostart;
    return h("p", { class: "s muted dev-auto" },
      on ? "Tests start a stopped emulator or simulator when they need it. " : "Tests do not start devices; use Start above. ",
      h("button", { class: "lk", onclick: () => answerAutostart(!on) }, on ? "Turn off" : "Turn on"));
  }
  return null;
}

function section() {
  const body = [];
  if (!fixData) {
    body.push(h("div", { class: "s muted" }, "Checking the devices for tests…"));
  } else {
    if (fixData.error) body.push(UI.notice("error", "Could not read the devices: " + fixData.error));
    const busy = !!(fixData.job && fixData.job.state === "running");
    for (const l of fixData.lines || []) body.push(...lineRows(l, busy));
    body.push(autostartRow());
    if (fixData.job) body.push(jobPanel(fixData.job));
  }
  return h("div", { class: "grp", id: "fix-section" },
    h("div", { class: "gh" }, icon("device"), "Devices for tests"),
    ...UI.kids(body),
    h("p", { class: "s muted fix-lock" }, icon("lock", "sm"), " Starting and setting up devices works only from this page. You cannot start them from Chat, so the AI cannot install things."));
}

// Each time the Device page draws (also after Check again), the offers are read again.
PAGES.deviceFix = () => {
  if (!fixBusy) loadFix();
  return section();
};
})();
