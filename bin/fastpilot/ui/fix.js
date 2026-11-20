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

function confirmFix(offer) {
  return new Promise((resolve) => {
    const done = (v) => { dlg.close(); dlg.remove(); resolve(v); };
    const cancel = h("button", { class: "btn", onclick: () => done(false) }, "Cancel");
    const dlg = h("dialog", { class: "hist-dlg", "aria-labelledby": "fix-dlg-t" },
      h("h2", { id: "fix-dlg-t" }, offer.area === "android" ? "Set up an Android emulator?" : "Install Chrome?"),
      h("p", {}, offer.confirm),
      offer.steps.length ? h("ol", { class: "fix-plan" }, offer.steps.map((s) => h("li", {}, s))) : null,
      h("div", { class: "btns tight" }, cancel, h("button", { class: "btn p", onclick: () => done(true) }, offer.button === "Install Chrome" ? "Install" : "Set up")));
    dlg.addEventListener("cancel", (e) => { e.preventDefault(); done(false); });
    document.body.append(dlg);
    dlg.showModal();
    cancel.focus();
  });
}

async function startFix(offer) {
  if (!(await confirmFix(offer))) return;
  try {
    const r = await fixPost({ action: "start", area: offer.area, confirmed: true });
    fixData = { ...fixData, job: r.job };
    fixDetails = false;
  } catch (e) {
    toast("Could not start: " + e.message);
  }
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
  const head = running ? (job.area === "android" ? "Setting up your emulator" : "Installing Chrome") : job.state === "done" ? "Done" : job.state === "stopped" ? "Stopped" : "Could not finish";
  return h("div", { class: "fix-job", role: "status", "aria-live": "polite" },
    h("div", { class: "fix-head" }, h("b", {}, head), running && current >= 0 ? h("span", { class: "pill" }, `Step ${current + 1} of ${n}`) : null),
    h("div", { class: "fix-bar" }, fill),
    h("ul", { class: "fix-steps" }, job.steps.map((s) => h("li", { class: s.state }, h("span", { "aria-hidden": "true" }, MARK[s.state] || "○"), " ", s.label, s.state === "running" ? "…" : ""))),
    job.message ? h("p", { class: job.state === "failed" ? "note-err" : "s" }, job.message) : null,
    h("div", { class: "btns tight fix-acts" },
      running ? h("button", { class: "btn sm", onclick: stopFix }, "Stop") : null,
      !running && job.state !== "done" ? h("button", { class: "btn sm", onclick: () => { const o = (fixData.offers || []).find((x) => x.area === job.area && x.button); if (o) startFix(o); else loadFix(); } }, "Try again") : null,
      h("button", { class: "btn sm", "aria-expanded": String(fixDetails), onclick: () => { fixDetails = !fixDetails; redraw(); } }, fixDetails ? "Hide details" : "Show details")),
    fixDetails ? h("pre", { class: "fix-log", tabindex: "0", "aria-label": "Details" }, job.log.length ? job.log.join("\n") : "Nothing yet.") : null);
}

function section() {
  const body = [];
  if (!fixData) {
    body.push(h("div", { class: "s muted" }, "Checking what fastpilot can set up…"));
  } else {
    if (fixData.error) body.push(h("div", { class: "err", role: "alert" }, "Could not read the fixes: " + fixData.error));
    const busy = fixData.job && fixData.job.state === "running";
    const offers = fixData.offers || [];
    if (!offers.length && !fixData.job) body.push(h("div", { class: "s muted" }, "Nothing to set up: Chrome and fastpilot's test phone are ready."));
    // While a fix runs, its own job panel stands for the offer.
    for (const o of offers.filter((x) => !(busy && fixData.job.area === x.area))) {
      body.push(h("div", { class: "row fix-row" },
        h("div", {}, h("div", { class: "t" }, h("span", { class: "dot " + (o.button ? "warn" : "fail") }), o.title), h("div", { class: "s" }, o.detail)),
        o.button ? h("button", { class: "btn sm p", disabled: busy, onclick: () => startFix(o) }, o.button) : null));
    }
    if (fixData.job) body.push(jobPanel(fixData.job));
  }
  return h("div", { class: "grp", id: "fix-section" },
    h("div", { class: "gh" }, icon("plug"), "Set up for tests"),
    ...body,
    h("p", { class: "s muted fix-lock" }, icon("lock", "sm"), " Fix buttons work only from this page. You cannot start them from Chat, so the AI cannot install things."));
}

// Each time the Device page draws (also after Check again), the offers are read again.
PAGES.deviceFix = () => {
  if (!fixBusy) loadFix();
  return section();
};
})();
