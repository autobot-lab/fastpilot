// fastpilot ui: Map (PF1, v0.6, design product-v11/01-pf1-app-map). Which parts of the app have a
// test, as a picture: every screen fastpilot found by walking through the app read-only, coloured
// by its tests, with a list view that has everything the graph has (for keyboards and screen
// readers). Screen titles and labels come from the app: they are set as text, never as HTML, and
// never read as instructions. Uses h(), icon(), api(), toast(), $top(), $body() and chatPrefill
// from app.js. Everything is inside one function; only PAGES.map is shared.
(() => {
"use strict";

let data = null; // { mapping, map, coverage, test_account }
let busy = false;
let view = "graph"; // graph | list
let selected = null; // screen id
let filter = "all"; // all | none | failing | tested
let search = "";
let zoom = 1, panX = 0, panY = 0;
let timer = null;
let form = { target: "", platform: "web", screens: 30, minutes: 10 };

const STATUS = {
  tested: ["✓ Has a test", "ok", "Has a passing test"],
  failing: ["✗ Test failing", "bad", "Test is failing"],
  none: ["No test yet", "none", "No test yet"],
};

async function load() {
  if (busy) return;
  busy = true;
  try { data = await api("/api/map"); } catch (e) { data = { error: e.message }; }
  busy = false;
  clearTimeout(timer);
  if (data && data.mapping) timer = setTimeout(load, 1000);
  if ((location.hash || "").startsWith("#/map")) draw();
}

function cov(id) {
  const c = (data.coverage && data.coverage.screens || []).find((s) => s.id === id);
  return c || { status: "none", tests: [] };
}

function when(ms) {
  if (!ms) return "";
  const d = new Date(ms), now = new Date();
  const t = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return d.toDateString() === now.toDateString() ? `today, ${t}` : `${d.toLocaleDateString()} ${t}`;
}

const platformName = (p) => ({ web: "Web", android: "Android emulator", ios: "iPhone simulator" })[p] || p;

async function startMap(extra) {
  const body = { action: "start", target: form.target, platform: form.platform, screens: Number(form.screens), minutes: Number(form.minutes), ...(extra || {}) };
  try { data = await api("/api/map", body); toast("Mapping started. Nothing will be typed in or submitted.", true); } catch (e) { toast(e.message); return; }
  selected = null;
  load();
}

async function stopMap() {
  try { data = await api("/api/map", { action: "stop" }); } catch (e) { toast(e.message); }
  load();
}

// Where a screen is, as shown: a local file by its name only (QA re-test Low 5), a site as is.
function place(location) {
  const l = String(location || "");
  if (l.startsWith("file://")) return l.split("#")[0].split("/").filter(Boolean).pop() || "local file";
  return l;
}

function makeTest(s) {
  const m = data.map;
  chatPrefill = `Make a test for the "${s.title}" screen of ${m.name || m.target} (${m.platform}, ${place(s.location)}). Open it the way the app map found it${s.path && s.path.length ? `: ${s.path.map((p) => p.name).join(" › ")}` : ""}.`;
  location.hash = "#/agents/default/chat";
}

// ---- empty state and the start form (design states 1, 7) ----
function startForm(note) {
  const target = h("input", { type: "text", value: form.target, placeholder: "https://staging.shop.example", "aria-describedby": "map-where-h", spellcheck: "false" });
  target.addEventListener("input", () => { form.target = target.value; go.disabled = !form.target.trim(); });
  const plat = h("select", { class: "sel", "aria-label": "Where the app runs" },
    [["web", "Web · Chrome"], ["android", "Android emulator"], ["ios", "iPhone simulator"]].map(([v, l]) => h("option", { value: v, selected: v === form.platform }, l)));
  plat.addEventListener("change", () => { form.platform = plat.value; target.placeholder = plat.value === "web" ? "https://staging.shop.example" : "com.example.app/.MainActivity"; });
  const num = (key, label, options) => {
    const s = h("select", { class: "sel", "aria-label": label }, options.map((v) => h("option", { value: v, selected: Number(form[key]) === v }, String(v))));
    s.addEventListener("change", () => { form[key] = Number(s.value); });
    return h("label", { class: "fld" }, h("span", { class: "l" }, label), s);
  };
  const go = h("button", { class: "btn p", disabled: !form.target.trim(), onclick: () => startMap() }, icon("map"), "Map this app");
  return h("section", { class: "grp map-start" },
    h("h2", { class: "gh" }, "map your app"),
    h("div", { class: "form ag-pad" },
      note,
      h("p", {}, "fastpilot walks through your app like a visitor and draws every screen it finds. Then you see which screens have a test."),
      h("label", { class: "fld" }, h("span", { class: "l" }, "Where should it start?"), target,
        h("span", { class: "s", id: "map-where-h" }, "A web address or a file of your site, or an app on the emulator (package/activity).")),
      h("div", { class: "map-limits" }, h("label", { class: "fld" }, h("span", { class: "l" }, "Where it runs"), plat),
        num("screens", "Up to how many screens", [10, 20, 30, 45, 60]), num("minutes", "Up to how many minutes", [5, 10, 15, 30])),
      h("div", { class: "app-note info", role: "note" }, h("b", {}, "Read-only. "),
        "It will not fill in forms, press Pay or Delete, or sign in. To map pages behind a sign-in, add a test account to ",
        h("a", { class: "lk", href: "#/memory" }, "Project notes"), " first (an email and a ${SECRET} name).",
        data && data.test_account ? h("span", {}, " A test account is in your project notes: it signs in with that once.") : null),
      h("div", { class: "btns" }, go)));
}

// ---- while mapping (design state 2) ----
function progress(m) {
  const secs = Math.max(0, Math.round((Date.now() - m.started_ms) / 1000));
  const clock = (s) => `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  const names = m.screens.map((s) => s.title).join(" · ");
  const blocked = m.skipped.filter((k) => k.why.startsWith("blocked") || k.why.startsWith("asks"));
  return h("section", { class: "grp map-run", "aria-label": "Mapping now" },
    h("h2", { class: "gh" }, h("span", { class: "spin sm", "aria-hidden": "true" }), "mapping"),
    h("div", { class: "ag-pad" },
      h("p", {}, h("b", {}, m.name || m.target), ` · ${clock(secs)} of up to ${clock(m.max_minutes * 60)}`),
      h("p", { role: "status" }, h("b", {}, `${m.screens.length} screen${m.screens.length === 1 ? "" : "s"} found`), ` · limit ${m.max_screens}`),
      names ? h("p", { class: "s" }, "✓ ", names) : null,
      m.current ? h("p", { class: "s" }, "▶ Looking at: ", m.current) : null,
      blocked.length ? h("p", { class: "s" }, `Skipped so far: ${blocked.length} button${blocked.length === 1 ? "" : "s"} (`, blocked.slice(0, 3).map((k) => `“${k.label}”`).join(", "), " is blocked by limits).") : null,
      h("div", { class: "btns" }, h("button", { class: "btn", onclick: stopMap }, icon("stop"), "Stop"), h("span", { class: "s" }, "Screens found so far are kept."))));
}

// ---- outcome notes (design states 3–9) ----
function outcome(m) {
  if (!m.state || m.state === "mapping") return null;
  const kind = { finished: ["ok", "✓"], stopped: ["info", "■"], limit: ["warn", "!"], could_not_open: ["bad", "!"], failed: ["bad", "✗"] }[m.state] || ["info", "i"];
  const old = m.finished_ms && Date.now() - m.finished_ms > 7 * 86400000;
  return [
    h("div", { class: `app-note ${kind[0]} map-out`, role: m.state === "finished" ? "status" : "alert" }, h("b", {}, kind[1], " "), m.message || "",
      m.state === "stopped" || m.state === "failed" ? [" ", h("button", { class: "btn sm", onclick: () => { form.target = m.target; form.platform = m.platform; startMap(); } }, "Continue mapping")] : null,
      m.wall ? [" ", h("a", { class: "lk", href: "#/memory" }, "Project notes")] : null),
    old ? h("div", { class: "app-note info", role: "status" }, h("b", {}, "i "), `Mapped ${Math.round((Date.now() - m.finished_ms) / 86400000)} days ago. Your app may have changed since. `,
      h("button", { class: "btn sm", onclick: () => { form.target = m.target; form.platform = m.platform; startMap(); } }, "Re-map the whole app")) : null,
  ];
}

// ---- the graph (design: graph, zoom/pan/fit, keyboard) ----
const NS = "http://www.w3.org/2000/svg";
function svg(tag, attrs, ...kids) {
  const el = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs || {})) if (v !== null && v !== undefined && v !== false) el.setAttribute(k, String(v));
  kids.flat().forEach((k) => { if (k !== null && k !== undefined) el.append(k); });
  return el;
}

function layout(m) {
  const depth = {}, order = [];
  if (!m.screens.length) return { pos: {}, w: 0, h: 0 };
  depth[m.screens[0].id] = 0; order.push(m.screens[0].id);
  for (let i = 0; i < order.length; i++) {
    for (const e of m.edges) if (e.from === order[i] && depth[e.to] === undefined) { depth[e.to] = depth[order[i]] + 1; order.push(e.to); }
  }
  m.screens.forEach((s) => { if (depth[s.id] === undefined) { depth[s.id] = 0; order.push(s.id); } });
  const cols = {};
  order.forEach((id) => { (cols[depth[id]] = cols[depth[id]] || []).push(id); });
  const W = 170, H = 70, GX = 70, GY = 26, pos = {};
  let maxRows = 0;
  Object.entries(cols).forEach(([d, ids]) => { maxRows = Math.max(maxRows, ids.length); ids.forEach((id, r) => { pos[id] = { x: Number(d) * (W + GX) + 20, y: r * (H + GY) + 20 }; }); });
  return { pos, W, H, w: (Object.keys(cols).length) * (W + GX) + 20, h: maxRows * (H + GY) + 20 };
}

function graph(m) {
  const { pos, W, H, w, h: ht } = layout(m);
  const ids = m.screens.map((s) => s.id);
  const g = svg("g", { transform: `translate(${panX} ${panY}) scale(${zoom})` });
  m.edges.forEach((e) => {
    const a = pos[e.from], b = pos[e.to];
    if (!a || !b) return;
    g.append(svg("line", { x1: a.x + W, y1: a.y + H / 2, x2: b.x, y2: b.y + H / 2, class: "map-edge", "marker-end": "url(#map-arrow)" }),
      svg("text", { x: (a.x + W + b.x) / 2, y: (a.y + b.y) / 2 + H / 2 - 4, class: "map-edge-l" }, e.label.slice(0, 18)));
  });
  m.screens.forEach((s, i) => {
    const p = pos[s.id], c = cov(s.id), st = STATUS[c.status] || STATUS.none;
    const node = svg("g", { class: `map-node ${st[1]}${selected === s.id ? " sel" : ""}`, transform: `translate(${p.x} ${p.y})`, tabindex: "0", role: "button", "aria-label": `${s.title}: ${st[0]}`, "data-id": s.id },
      svg("rect", { width: W, height: H, rx: 6 }),
      svg("text", { x: 10, y: 24, class: "map-t" }, s.title.length > 22 ? s.title.slice(0, 21) + "…" : s.title),
      svg("text", { x: 10, y: 48, class: "map-st" }, (s.sign_in ? "Sign-in · " : "") + st[0]));
    node.addEventListener("click", () => { selected = s.id; draw(); });
    node.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" || ev.key === " ") { ev.preventDefault(); selected = s.id; draw(); focusNode(s.id); }
      const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[ev.key];
      if (step) { ev.preventDefault(); const next = ids[(i + step + ids.length) % ids.length]; focusNode(next); }
    });
    g.append(node);
  });
  const box = svg("svg", { class: "map-svg", viewBox: `0 0 ${Math.max(w, 400)} ${Math.max(ht, 240)}`, role: "group", "aria-label": "Map of the app's screens" },
    svg("defs", {}, svg("marker", { id: "map-arrow", viewBox: "0 0 10 10", refX: 9, refY: 5, markerWidth: 7, markerHeight: 7, orient: "auto-start-reverse" }, svg("path", { d: "M0,0 L10,5 L0,10 z", class: "map-arrow" }))),
    g);
  let drag = null;
  box.addEventListener("pointerdown", (ev) => { if (ev.target.closest && ev.target.closest(".map-node")) return; drag = { x: ev.clientX - panX, y: ev.clientY - panY }; box.setPointerCapture(ev.pointerId); });
  box.addEventListener("pointermove", (ev) => { if (!drag) return; panX = ev.clientX - drag.x; panY = ev.clientY - drag.y; g.setAttribute("transform", `translate(${panX} ${panY}) scale(${zoom})`); });
  box.addEventListener("pointerup", () => { drag = null; });
  box.addEventListener("wheel", (ev) => { ev.preventDefault(); zoom = Math.min(3, Math.max(0.3, zoom * (ev.deltaY < 0 ? 1.1 : 0.9))); g.setAttribute("transform", `translate(${panX} ${panY}) scale(${zoom})`); }, { passive: false });
  const zb = (label, text, f) => h("button", { class: "btn sm", "aria-label": label, onclick: () => { f(); draw(); } }, text);
  return h("div", { class: "map-graph" },
    h("div", { class: "map-zoom" }, zb("Zoom in", "+", () => { zoom = Math.min(3, zoom * 1.2); }), zb("Zoom out", "−", () => { zoom = Math.max(0.3, zoom / 1.2); }), zb("Fit the map", "Fit", () => { zoom = 1; panX = 0; panY = 0; })),
    box,
    h("p", { class: "s muted" }, "Drag to move · scroll or +/− to zoom · arrow keys move between screens"));
}

function focusNode(id) {
  requestAnimationFrame(() => document.querySelector(`.map-node[data-id="${id}"]`)?.focus());
}

// ---- the selected screen (design: panel) ----
function panel(m) {
  const s = m.screens.find((x) => x.id === selected);
  if (!s) return h("aside", { class: "map-side" }, h("p", { class: "s muted" }, "Choose a screen to see its tests, where it leads and its picture."));
  const c = cov(s.id), st = STATUS[c.status] || STATUS.none;
  const title = (id) => (m.screens.find((x) => x.id === id) || {}).title || "?";
  const from = m.edges.filter((e) => e.to === s.id).map((e) => title(e.from));
  const to = m.edges.filter((e) => e.from === s.id).map((e) => title(e.to));
  const testRow = (t) => h("li", {}, h("span", { class: t.status === "failing" ? "bad" : "ok" }, t.status === "failing" ? "✗ " : "✓ "), t.title,
    t.status === "not_run" ? h("span", { class: "s muted" }, " · not run yet") : t.status === "failing" ? h("span", { class: "s" }, " · failing") : null);
  const skipped = m.skipped.filter((k) => k.screen === s.title);
  return h("aside", { class: "map-side", "aria-label": "Selected screen" },
    h("h2", { class: "app-sec-t" }, "Selected screen"),
    h("h3", {}, s.title), h("p", { class: `map-pill ${st[1]}` }, st[0]),
    s.thumb ? h("img", { class: "map-thumb", src: `/api/map/thumb?target=${encodeURIComponent(m.target)}&id=${encodeURIComponent(s.id)}`, alt: `Picture of the ${s.title} screen`, loading: "lazy" }) : null,
    h("p", { class: "s muted" }, place(s.location)),
    h("h4", {}, "Tests that reach this screen"),
    c.tests.length ? h("ul", { class: "map-tests" }, c.tests.map(testRow)) : h("p", { class: "s" }, "None yet."),
    h("p", { class: "s" }, `You can get here from: ${from.length ? from.join(", ") : "the start"}. You can go from here to: ${to.length ? to.join(", ") : "nothing found"}.`),
    skipped.length ? h("p", { class: "s" }, "Not pressed here: ", skipped.map((k) => `“${k.label}” (${k.why})`).join(", "), ".") : null,
    h("div", { class: "btns" },
      h("button", { class: "btn p", onclick: () => makeTest(s) }, "Make a test for this screen"),
      m.platform === "web" && s.location.includes("://") ? h("button", { class: "btn", onclick: () => { form.target = s.location; form.platform = m.platform; startMap(); } }, "Re-map from here") : null),
    h("p", { class: "s muted" }, "“Make a test” opens Chat with a ready request. You still see the Save test card and decide."));
}

// ---- the list view (design: a real table, problems first) ----
function list(m) {
  const order = { failing: 0, none: 1, tested: 2 };
  const counts = { all: m.screens.length, none: 0, failing: 0, tested: 0 };
  m.screens.forEach((s) => { counts[cov(s.id).status] += 1; });
  const rows = m.screens
    .filter((s) => filter === "all" || cov(s.id).status === filter)
    .filter((s) => !search || s.title.toLowerCase().includes(search.toLowerCase()))
    .sort((a, b) => order[cov(a.id).status] - order[cov(b.id).status]);
  const title = (id) => (m.screens.find((x) => x.id === id) || {}).title || "?";
  const q = h("input", { type: "search", value: search, placeholder: "Search screens", "aria-label": "Search screens" });
  q.addEventListener("input", () => { search = q.value; const t = document.getElementById("map-table"); if (t) t.replaceWith(table()); });
  const chip = (k, label) => h("button", { class: "chip" + (filter === k ? " on" : ""), "aria-pressed": String(filter === k), onclick: () => { filter = k; draw(); } }, `${label} ${counts[k]}`);
  function table() {
    return h("table", { class: "map-table", id: "map-table" },
      h("caption", {}, "Screens, their test status and where they lead"),
      h("thead", {}, h("tr", {}, ["Screen", "Test", "Tests that reach it", "Leads to", "Actions"].map((c) => h("th", { scope: "col" }, c)))),
      h("tbody", {}, rows.map((s) => {
        const c = cov(s.id), st = STATUS[c.status] || STATUS.none;
        const to = m.edges.filter((e) => e.from === s.id).map((e) => title(e.to));
        return h("tr", {},
          h("th", { scope: "row" }, s.title, s.sign_in ? h("span", { class: "s muted" }, " (sign-in)") : null),
          h("td", {}, h("span", { class: `map-pill ${st[1]}` }, st[0])),
          h("td", {}, c.tests.length ? c.tests.map((t) => t.title).join(" · ") : "None"),
          h("td", {}, to.length ? to.join(" · ") : "Nothing found"),
          h("td", {}, h("div", { class: "btns tight" },
            c.status === "none" ? h("button", { class: "btn sm p", onclick: () => makeTest(s) }, "Make a test") : null,
            h("button", { class: "btn sm", onclick: () => { selected = s.id; view = "graph"; draw(); focusNode(s.id); } }, "Open"))));
      })));
  }
  return h("div", { class: "map-list" },
    h("div", { class: "map-filters" }, q, chip("all", "All"), chip("none", "No test"), chip("failing", "Failing"), chip("tested", "Has a test")),
    table(),
    h("p", { class: "s muted" }, "The list has everything the graph has. It is the version for keyboards and screen readers: Tab moves between rows and buttons."));
}

function draw() {
  const m = data && data.map;
  const viewBtn = (k, label) => h("button", { class: "btn sm" + (view === k ? " p" : ""), "aria-pressed": String(view === k), onclick: () => { view = k; draw(); } }, label);
  UI.pageHeader({ icon: "map", title: "Map", sub: "Which parts of your app have a test",
    actions: m && m.screens.length && !data.mapping ? [viewBtn("graph", "Graph"), viewBtn("list", "List"),
      h("button", { class: "btn", onclick: () => { form.target = m.target; form.platform = m.platform; startMap(); } }, "Re-map the whole app"),
      h("button", { class: "btn quiet", onclick: () => { data = { ...data, map: null }; draw(); } }, "Map another app")] : null });
  if (!data) { $body().replaceChildren(h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" }), h("b", {}, "Reading the map…"))); load(); return; }
  if (data.error) { $body().replaceChildren(UI.notice("error", "Could not read the map: " + data.error)); return; }
  if (data.mapping) { $body().replaceChildren(progress(m)); return; }
  if (!m || !m.screens.length) { $body().replaceChildren(startForm(m ? outcome(m) : null)); return; }
  const c = data.coverage || { tested: 0, failing: 0, total: m.screens.length };
  const pct = c.total ? Math.round((100 * c.tested) / c.total) : 0;
  const summary = h("div", { class: "map-sum" },
    h("p", {}, h("b", {}, m.name || m.target), h("span", { class: "s muted" }, ` ${platformName(m.platform)} · mapped ${when(m.finished_ms || m.started_ms)} · ${m.screens.length} screens`)),
    h("p", {}, h("b", {}, `${c.tested} of ${c.total} screens have a test`), c.failing ? h("span", { class: "bad" }, ` · ${c.failing} of them ${c.failing === 1 ? "is" : "are"} failing`) : null),
    h("div", { class: "map-bar", role: "img", "aria-label": `${pct}% of screens have a test` }, h("span", { class: "map-bar-fill", "data-pct": String(pct) })),
    h("ul", { class: "map-legend", "aria-label": "Legend" }, Object.values(STATUS).map(([, cls, text]) => h("li", {}, h("span", { class: `map-key ${cls}`, "aria-hidden": "true" }), text))),
    h("p", { class: "s muted" }, "Screens are found by exploring. ",
      m.platform === "web" ? (m.blocked ? `${m.blocked} request${m.blocked === 1 ? "" : "s"} that would have changed something ${m.blocked === 1 ? "was" : "were"} blocked. ` : "Nothing was submitted. ") : "Only links and buttons that move around were pressed. ",
      m.signed_in ? "It signed in with the test account from your project notes; nothing else was typed in. " : "Nothing was typed in. ",
      "Pictures show the app as it looked while mapping. The map is kept only on this computer, in fastpilot's own folder."));
  $body().replaceChildren(h("div", { class: "map" }, outcome(m), summary,
    view === "list" ? list(m) : h("div", { class: "map-main" }, graph(m), panel(m))));
  const fill = document.querySelector(".map-bar-fill");
  if (fill) fill.style.width = `${pct}%`;
}

PAGES.map = () => { if (!data) load(); draw(); };
window.addEventListener("hashchange", () => { if ((location.hash || "") === "#/map") load(); });
})();
