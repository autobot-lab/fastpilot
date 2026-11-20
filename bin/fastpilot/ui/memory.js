// fastpilot ui: Memory (D6, v0.4). What fastpilot has learned about each test, in words, and the
// two things a person can do: Set aside / Bring back, and Forget. Stage 2: project notes, written
// only here by the person (never by Chat or a tested page). Uses h(), icon(), api(), toast()
// from app.js. Values from the server are always set as text, never as HTML. Everything is inside
// one function, so no name here meets one in app.js; only PAGES.memory is shared.
(() => {
"use strict";

let memData = null; // { tests: [...], mode }
let memBusy = false;
let memPick = null; // id of the test in the side panel
let memTab = "tests"; // tests | notes
let notesData = null; // { notes, groups, limits, used }
let noteForm = { group: "addresses", text: "", id: null }; // the add / edit form
let noteError = null; // { kind, message }

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
  if ((location.hash || "").startsWith("#/memory")) render();
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

function memTop(sub) {
  const tab = (key, label, n) => h("button", { class: "hist-tab" + (memTab === key ? " on" : ""), role: "tab", "aria-selected": String(memTab === key),
    onclick: () => { memTab = key; render(); } }, label, n === undefined ? null : h("span", { class: "cnt" }, String(n)));
  $top().replaceChildren(icon("memory"), h("h1", { tabindex: "-1" }, "Memory"), h("span", { class: "muted hist-sub" }, sub),
    h("div", { class: "hist-tabs", role: "tablist" }, tab("tests", "Tests"), tab("notes", "Project notes", notesData ? notesData.notes.length : undefined)));
}

function render() {
  if (memTab === "notes") pageNotes(); else pageMemory();
}

// ---- project notes (D6 stage 2) ----

const QUESTIONS = [
  ["Where is the staging site?", "addresses", "Staging site: "],
  ["Which test account should I use?", "accounts", "Test account: sign in as "],
  ["Does login use a code?", "how", "Login uses a one-time code: "],
  ["What must never be tested?", "never", "Never "],
];

let notesBusy = false;
async function loadNotes() {
  if (notesBusy) return;
  notesBusy = true;
  try {
    notesData = await api("/api/notes");
  } catch (e) {
    notesData = { notes: [], groups: [], limits: { notes: 20, chars: 300, total: 2000 }, used: 0, error: e.message };
  }
  notesBusy = false;
  if ((location.hash || "").startsWith("#/memory") && memTab === "tests") memTop("What fastpilot has learned about your tests");
  if ((location.hash || "").startsWith("#/memory") && memTab === "notes") pageNotes();
}

// The server answers a refused note with {error, kind}; api() would only keep the text.
async function notesPost(body) {
  let r;
  try {
    r = await fetch("/api/notes", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  } catch (_) {
    throw new Error("the fastpilot ui server is not answering; start `fastpilot ui` again in Terminal");
  }
  const text = await r.text();
  let v = null;
  try { v = JSON.parse(text); } catch (_) { v = null; }
  if (!r.ok) return { error: (v && v.error) || text.replace(/^fastpilot ui: /, "").trim(), kind: (v && v.kind) || "other" };
  return v;
}

async function saveNote() {
  const body = noteForm.id ? { action: "edit", id: noteForm.id, group: noteForm.group, text: noteForm.text } : { action: "add", group: noteForm.group, text: noteForm.text };
  let r;
  try { r = await notesPost(body); } catch (e) { toast(e.message); return; }
  if (r.error) {
    noteError = { kind: r.kind, message: r.error };
    pageNotes();
    document.querySelector(".note-err")?.focus();
    return;
  }
  notesData = r;
  noteError = null;
  noteForm = { group: noteForm.group, text: "", id: null };
  toast("✓ " + r.ok, true);
  pageNotes();
}

function confirmRemove(note) {
  return new Promise((resolve) => {
    const done = (v) => { dlg.close(); dlg.remove(); resolve(v); };
    const keep = h("button", { class: "btn", onclick: () => done(false) }, "Keep it");
    const dlg = h("dialog", { class: "hist-dlg", "aria-labelledby": "note-dlg-t" },
      h("h2", { id: "note-dlg-t" }, `Remove "${noteTitle(note.text)[0]}"?`),
      h("p", {}, "Agents will stop using it from the next run."),
      h("div", { class: "btns tight" }, keep, h("button", { class: "btn p danger-fill", onclick: () => done(true) }, "Remove")));
    dlg.addEventListener("cancel", (e) => { e.preventDefault(); done(false); });
    document.body.append(dlg);
    dlg.showModal();
    keep.focus();
  });
}

async function removeNote(note) {
  if (!(await confirmRemove(note))) return;
  let r;
  try { r = await notesPost({ action: "remove", id: note.id }); } catch (e) { toast(e.message); return; }
  if (r.error) { toast(r.error); return; }
  notesData = r;
  if (noteForm.id === note.id) noteForm = { group: noteForm.group, text: "", id: null };
  toast(r.ok, true);
  pageNotes();
}

// "Staging shop: https://…" shows as a title and a line; a note without a short label is one line.
function noteTitle(text) {
  const i = text.indexOf(": ");
  return i > 0 && i <= 40 ? [text.slice(0, i), text.slice(i + 2)] : [text, ""];
}

function notesPanel() {
  const L = notesData.limits;
  const n = noteForm.text.length;
  const full = !noteForm.id && notesData.notes.length >= L.notes;
  const groups = h("div", { class: "note-groups", role: "radiogroup", "aria-label": "Group" }, notesData.groups.map((g) =>
    h("button", { class: "hist-f" + (noteForm.group === g.key ? " on" : ""), role: "radio", "aria-checked": String(noteForm.group === g.key),
      onclick: () => { noteForm.group = g.key; pageNotes(); } }, g.label)));
  const area = h("textarea", { class: "note-text", rows: 3, maxlength: String(L.chars), "aria-label": "Note", placeholder: "What should every agent know?",
    oninput: (e) => { noteForm.text = e.target.value; noteError = null; const c = document.querySelector(".note-count"); if (c) c.textContent = `${e.target.value.length} of ${L.chars} characters`; } });
  area.value = noteForm.text;
  const err = noteError
    ? h("div", { class: "note-err", role: "alert", tabindex: "-1" }, noteError.message,
      noteError.kind === "secret" ? h("div", { class: "btns tight note-acts" },
        h("a", { class: "btn sm p", href: "#/settings/secret" }, "Save as a secret instead"),
        h("button", { class: "btn sm", onclick: () => { noteError = null; pageNotes(); document.querySelector(".note-text")?.focus(); } }, "Remove the password")) : null)
    : null;
  return h("aside", { class: "mem-panel" },
    h("div", { class: "mem-k" }, noteForm.id ? "Edit note" : "Add a note"),
    full ? h("p", { class: "note-err" }, `${L.notes} of ${L.notes} notes. Remove or shorten a note to add another.`) : null,
    h("div", { class: "s muted" }, "Group"), groups, area,
    h("div", { class: "s muted note-count" }, `${n} of ${L.chars} characters`),
    err,
    h("div", { class: "btns tight mem-btns" },
      h("button", { class: "btn sm p", disabled: full || !noteForm.text.trim(), onclick: saveNote }, "Save note"),
      noteForm.id || noteForm.text ? h("button", { class: "btn sm", onclick: () => { noteForm = { group: noteForm.group, text: "", id: null }; noteError = null; pageNotes(); } }, "Cancel") : null),
    noteForm.id ? null : [h("div", { class: "mem-k" }, "Or start from a question"),
      h("div", { class: "note-qs" }, QUESTIONS.map(([q, g, start]) => h("button", { class: "chip preset", onclick: () => { noteForm = { group: g, text: start, id: null }; noteError = null; pageNotes(); const t = document.querySelector(".note-text"); t?.focus(); t?.setSelectionRange(start.length, start.length); } }, q)))],
    h("p", { class: "note-warn" }, h("b", {}, "Notes are not hidden. "), "Never write a password, a token or a code here. Save those as ", h("a", { class: "lk", href: "#/settings/secret" }, "Secrets"), " and only mention the name."),
    h("p", { class: "s muted" }, "Every agent in this project reads these notes when a run starts. Runs already started keep the notes they began with."),
    h("p", { class: "s muted" }, "Kept on this computer for this project folder, not in the project's files: a project you download cannot bring notes of its own."));
}

// The page's CSP has no inline styles: the width is set through the DOM, not a style attribute.
function bar(pct, label) {
  const fill = h("i", {});
  fill.style.width = pct + "%";
  return h("div", { class: "bar", role: "img", "aria-label": label }, fill);
}

function pageNotes() {
  memTop("What fastpilot knows about your project");
  if (!notesData) {
    $body().replaceChildren(h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" }), h("b", {}, "Reading notes…")));
    loadNotes();
    return;
  }
  const L = notesData.limits;
  const list = notesData.notes;
  const pct = Math.min(100, Math.round((Math.max(list.length / L.notes, notesData.used / L.total)) * 100));
  const head = h("div", { class: "grp note-head" },
    h("div", {}, h("b", {}, "Project notes"), h("div", { class: "s muted" }, "Short facts that every agent in this project reads before it tests. They save you from repeating yourself in Chat.")),
    h("div", { class: "note-meter" }, h("div", { class: "s" }, `${list.length} of ${L.notes} notes · ${notesData.used.toLocaleString("en-GB")} of ${L.total.toLocaleString("en-GB")} characters`),
      bar(pct, `${list.length} of ${L.notes} notes, ${notesData.used} of ${L.total} characters used`)));
  const groups = list.length
    ? h("div", { class: "grp" }, notesData.groups.filter((g) => list.some((x) => x.group === g.key)).map((g) => [
      h("div", { class: "gh" }, g.label),
      list.filter((x) => x.group === g.key).map((x) => {
        const [title, rest] = noteTitle(x.text);
        return h("div", { class: "note-row" + (noteForm.id === x.id ? " on" : "") + (x.problem ? " off" : "") },
          h("div", {}, h("div", { class: "row-name" }, title), rest ? h("div", { class: "s muted" }, rest) : null,
            x.problem ? h("div", { class: "note-off" }, x.problem) : null),
          h("div", { class: "btns tight" },
            h("button", { class: "btn sm", "aria-label": `Edit note: ${title}`, onclick: () => { noteForm = { group: x.group, text: x.text, id: x.id }; noteError = null; pageNotes(); document.querySelector(".note-text")?.focus(); } }, "Edit"),
            h("button", { class: "btn sm", "aria-label": `Remove note: ${title}`, onclick: () => removeNote(x) }, "Remove")));
      })]))
    : h("div", { class: "grp" }, h("div", { class: "empty" }, h("b", {}, "No notes yet"),
      h("span", {}, "Add the things you keep repeating: where the staging site is, which test account to use, how login works."),
      h("button", { class: "btn p", onclick: () => document.querySelector(".note-text")?.focus() }, "Add your first note")));
  $body().replaceChildren(...(notesData.error ? [h("div", { class: "err", role: "alert" }, "Could not read the notes: " + notesData.error)] : []),
    ...(notesData.damaged ? [h("div", { class: "err", role: "alert" }, "The notes file for this project cannot be read, so agents get no notes (and no \"Never do\" rules) until it is fixed. Nothing here can change it; ask for help.")] : []),
    h("div", { class: "mem-wrap" }, h("div", {}, head, groups), notesPanel()));
}

function pageMemory() {
  memTop("What fastpilot has learned about your tests");
  if (!notesData) loadNotes();
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
  notesData = null;
  render();
};
})();
