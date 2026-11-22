// fastpilot ui: the apps being tested (E6b, v0.7). The App switcher above every page, the Add app
// dialog (also at #/apps/add?app=<id>&kind=<web|android|ios>, which S1 and S8 link to), and
// Settings › Your apps. Built on UI.* (components.js) and /api/apps (apps.rs).
(() => {
"use strict";

const KIND = { web: ["A website", "Opens in Chrome", "Website"], android: ["An Android app", "On an emulator", "Android app"], ios: ["An iPhone app", "On a simulator", "iPhone app"] };
let data = null; // { current, apps }
let busy = false;
let closing = false; // the Add dialog is going back from #/apps/add; do not open it again meanwhile

async function load() {
  if (busy) return;
  busy = true;
  try { data = await api("/api/apps"); } catch (e) { data = { error: e.message, apps: [], current: null }; }
  busy = false;
  bar();
  if ((location.hash || "").startsWith("#/settings/apps")) render();
}

const kinds = (a) => (a.kinds || []).map((k) => KIND[k][2]).join(" + ");
const initials = (name) => name.split(/\s+/).filter(Boolean).slice(0, 2).map((w) => w[0].toUpperCase()).join("") || "A";

/// The bar above every page: the current app and its kind; the list, Add app and Manage apps.
function bar() {
  const host = document.getElementById("appbar");
  if (!host) return;
  if (!data) { load(); return; }
  if (!data.apps.length) {
    host.replaceChildren(h("button", { type: "button", class: "btn sm", onclick: () => addDialog({}) }, icon("plus", "sm"), "Add your app"));
    return;
  }
  const cur = data.apps.find((a) => a.id === data.current);
  const list = h("ul", { class: "appmenu", role: "menu", "aria-label": "Your apps", hidden: true },
    data.apps.map((a) => h("li", { role: "none" }, h("button", { type: "button", role: "menuitemradio", "aria-checked": String(a.id === data.current), class: "appitem", onclick: () => use(a.id) },
      h("span", { class: "av", "aria-hidden": "true" }, initials(a.name)),
      h("span", { class: "appname" }, h("b", {}, a.name), a.repo ? h("span", { class: "tag" }, "Repo") : null, h("small", {}, kinds(a)))))),
    h("li", { role: "none" }, h("button", { type: "button", role: "menuitem", class: "appitem", onclick: () => { close(); addDialog({}); } }, icon("plus", "sm"), "Add app")),
    h("li", { role: "none" }, h("a", { role: "menuitem", class: "appitem", href: "#/settings/apps", onclick: close }, "Manage apps…")));
  const btn = h("button", { type: "button", class: "appbtn", "aria-haspopup": "menu", "aria-expanded": "false", onclick: () => (list.hidden ? open() : close()) },
    h("span", { class: "av", "aria-hidden": "true" }, initials(cur ? cur.name : "?")),
    h("span", { class: "appname" }, h("b", {}, cur ? cur.name : "Choose an app"), h("small", {}, cur ? kinds(cur) : `${data.apps.length} apps`)), icon("chev", "sm"));
  function open() { list.hidden = false; btn.setAttribute("aria-expanded", "true"); list.querySelector("button")?.focus(); }
  function close() { list.hidden = true; btn.setAttribute("aria-expanded", "false"); }
  list.addEventListener("keydown", (e) => { if (e.key === "Escape") { close(); btn.focus(); } });
  host.replaceChildren(h("div", { class: "appswitch" }, btn, list));
}

async function use(id) {
  try { await api("/api/apps", { action: "use", id }); } catch (e) { toast(e.message); }
  data = null;
  await load();
  render();
}

/// "staging.shop.example" → "Shop staging"; "com.shop.app" → "Shop".
function suggest(kind, value) {
  const cap = (w) => (w ? w[0].toUpperCase() + w.slice(1) : "");
  if (kind === "web") {
    let host;
    try { host = new URL(value).hostname; } catch (_) { return ""; }
    if (!host || host === "localhost" || /^[\d.]+$/.test(host)) return host === "localhost" ? "Local site" : "";
    const parts = host.split(".").filter((p) => p !== "www");
    const words = parts.length > 1 ? parts.slice(0, -1) : parts;
    return words.length > 1 ? `${cap(words[1])} ${words[0]}` : cap(words[0]);
  }
  const segs = value.split(".").filter(Boolean);
  const generic = ["app", "android", "ios", "mobile", "client"];
  const w = segs.length > 1 && generic.includes(segs[segs.length - 1]) ? segs[segs.length - 2] : segs[segs.length - 1];
  return cap(w || "");
}

/// The Add app dialog. `app`: add a platform to that app; `kind`: preselected.
function addDialog({ app, kind }) {
  const existing = app && data && data.apps.find((a) => a.id === app);
  let pick = KIND[kind] ? kind : "web";
  let named = false;
  const err = (id) => h("p", { class: "err-line", id, role: "alert", hidden: true });
  const where = h("input", { type: "text", id: "app-where", autocomplete: "off", spellcheck: "false", "aria-describedby": "app-where-err app-where-hint" });
  const name = h("input", { type: "text", id: "app-name", autocomplete: "off", maxlength: "60", "aria-describedby": "app-name-err" });
  const whereErr = err("app-where-err"), nameErr = err("app-name-err"), formErr = err("app-form-err");
  const label = h("label", { for: "app-where", class: "l" });
  const hint = h("p", { class: "s muted", id: "app-where-hint" });
  const choice = h("div", { class: "seg", role: "radiogroup", "aria-label": "What do you want to test?" },
    Object.entries(KIND).filter(([k]) => !existing || !(existing.kinds || []).includes(k)).map(([k, [t, sub]]) =>
      h("button", { type: "button", role: "radio", "data-v": k, "aria-checked": String(k === pick), onclick: () => { pick = k; draw(); where.focus(); } }, h("b", {}, t), h("small", {}, sub))));
  if (!choice.querySelector(`[data-v="${pick}"]`)) pick = choice.querySelector("[data-v]")?.dataset.v || "web";
  function draw() {
    choice.querySelectorAll("[role=radio]").forEach((b) => b.setAttribute("aria-checked", String(b.dataset.v === pick)));
    label.textContent = pick === "web" ? "Address of the website" : `App id of the ${KIND[pick][2]}`;
    where.placeholder = pick === "web" ? "https://staging.shop.example" : "com.shop.app";
    hint.textContent = pick === "web" ? "Use a test or staging site, not your live shop." : "The app id looks like com.company.app. Your developers know it.";
    whereErr.hidden = true;
  }
  where.addEventListener("input", () => {
    whereErr.hidden = true;
    if (!named && !existing) name.value = suggest(pick, where.value.trim());
  });
  name.addEventListener("input", () => { named = true; nameErr.hidden = true; });
  const show = (el, text) => { el.textContent = text; el.hidden = false; };
  const add = h("button", { type: "submit", class: "btn p" }, existing ? `Add ${KIND[pick][2]}` : "Add app");
  const cancel = h("button", { type: "button", class: "btn", onclick: () => done() }, "Cancel");
  const form = h("form", { class: "form", novalidate: true },
    h("h2", { id: "app-dlg-t" }, existing ? `Add to ${existing.name}` : "Add an app"),
    h("p", { class: "s muted" }, existing ? "Tests of this app can then run on it too." : "Takes about a minute. Adding an app tests nothing and changes nothing on your computer."),
    h("fieldset", { class: "fld" }, h("legend", { class: "l" }, "What do you want to test?"), choice),
    h("div", { class: "fld" }, label, where, hint, whereErr),
    existing ? null : h("div", { class: "fld" }, h("label", { for: "app-name", class: "l" }, "Name"), name, nameErr),
    formErr,
    h("div", { class: "btns" }, add, cancel, existing ? null : h("span", { class: "s muted" }, "You can remove an app later without losing your tests")));
  const dlg = h("dialog", { class: "cdlg appdlg", "aria-labelledby": "app-dlg-t" }, form);
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const value = where.value.trim();
    if (!value) { show(whereErr, pick === "web" ? "Paste the address of the website, for example https://staging.shop.example." : "Enter the app id, for example com.shop.app."); where.focus(); return; }
    const targets = Object.assign({}, existing ? existing.targets : {}, { [pick]: value });
    try {
      const r = existing ? await api("/api/apps", { action: "update", id: existing.id, targets }) : await api("/api/apps", { action: "add", name: name.value, targets });
      done();
      data = null;
      await load();
      render();
      toast(existing ? `${KIND[pick][2]} added to ${r.app.name}.` : `${r.app.name} was added${data && data.current === r.app.id ? ". It is now your current app" : ""}. Tell Chat what to test, for example "check the login".`, true);
    } catch (x) {
      const m = x.message;
      if (/name/i.test(m) && !existing) { show(nameErr, m); name.focus(); } else if (/address|app id|website|user name/i.test(m)) { show(whereErr, m); where.focus(); } else show(formErr, m);
    }
  });
  dlg.addEventListener("cancel", (e) => { e.preventDefault(); done(); });
  const back = document.activeElement;
  function done() {
    dlg.close();
    dlg.remove();
    if ((location.hash || "").startsWith("#/apps/add")) {
      closing = true;
      window.addEventListener("hashchange", () => { closing = false; }, { once: true });
      if (history.length > 1) history.back(); else location.replace("#/settings/apps");
    }
    if (back && back.isConnected && back.focus) back.focus();
  }
  draw();
  document.body.append(dlg);
  dlg.showModal();
  where.focus();
}

/// Settings › Your apps.
function page() {
  if (!data) { load(); return h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" }), h("b", {}, "Reading your apps…")); }
  if (data.error) return UI.notice("error", "Could not read your apps: " + data.error);
  const rows = data.apps.map((a) => h("tr", { "data-id": a.id },
    h("th", { scope: "row" }, h("span", { class: "av", "aria-hidden": "true" }, initials(a.name)), " ", a.name, a.repo ? h("span", { class: "tag" }, " Repo") : null, a.id === data.current ? h("span", { class: "tag" }, " Current") : null),
    h("td", {}, kinds(a)),
    h("td", {}, a.repo ? `In the folder ${a.repo}: tests live in this repo` : "In your fastpilot workspace"),
    h("td", {}, h("span", { class: "btns tight" },
      h("button", { type: "button", class: "btn sm", onclick: () => rename(a) }, "Rename"),
      h("button", { type: "button", class: "btn sm", onclick: () => remove(a) }, a.repo ? "Remove from list…" : "Remove…")))));
  return h("div", { class: "page" },
    h("h2", {}, "Your apps"),
    h("p", { class: "s muted" }, "Each app keeps its own tests, schedules, map and Memory. Removing an app from the list never deletes your files."),
    h("div", { class: "btns" }, h("button", { type: "button", class: "btn p", onclick: () => addDialog({}) }, icon("plus", "sm"), "Add app")),
    data.apps.length
      ? h("div", { class: "tbl-wrap" }, h("table", { class: "tbl" }, h("caption", { class: "sr" }, "Your apps"),
        h("thead", {}, h("tr", {}, ["App", "Kind", "Where its tests are saved", ""].map((t) => h("th", { scope: "col" }, t)))), h("tbody", {}, rows)))
      : UI.empty({ icon: "tests", title: "No app yet", text: "Add the website or phone app you want to test. fastpilot keeps its tests, schedules and results under it.", action: { label: "Add app", icon: "plus", onclick: () => addDialog({}) } }),
    older(),
    data.apps.some((a) => a.repo) ? UI.notice("info", "\"Repo\" is an app in a code folder you set up with fastpilot init. Its tests are files next to your code, so your team can review them in git. fastpilot never deletes a repo's files.") : null);
}

/// "Look for older fastpilot folders?": searched only on the click; shown, then added with one more
/// click. Nothing is moved, copied or changed.
let found = null;
function older() {
  if (found && found.length) {
    return h("section", { class: "grp", "aria-label": "Older fastpilot folders" },
      h("div", { class: "gh" }, `older fastpilot folders · ${found.length}`),
      h("ul", { class: "s" }, found.map((f) => h("li", {}, h("b", {}, f.name), " ", h("span", { class: "muted" }, f.path)))),
      h("div", { class: "btns" }, h("button", { type: "button", class: "btn p", onclick: async () => {
        try { const r = await api("/api/apps", { action: "add_found", paths: found.map((f) => f.path) }); toast(`${r.added.length === 1 ? "1 folder is" : `${r.added.length} folders are`} now in your apps. Nothing was moved.`, true); } catch (e) { toast(e.message); }
        found = null; data = null; await load(); render();
      } }, "Show them as apps"), h("button", { type: "button", class: "btn", onclick: () => { found = null; render(); } }, "Not now")),
      h("p", { class: "s muted" }, "Nothing is moved, copied or changed: they are listed as Repo apps."));
  }
  return h("div", { class: "btns" }, h("button", { type: "button", class: "btn", onclick: async () => {
    try { const r = await api("/api/apps", { action: "find_older" }); found = r.found; if (!found.length) toast("No older fastpilot folders were found in your home folder.", true); } catch (e) { toast(e.message); }
    render();
  } }, "Look for older fastpilot folders"), h("span", { class: "s muted" }, "Searches your home folder for folders set up before. Nothing is moved."));
}

async function rename(a) {
  const input = h("input", { type: "text", maxlength: "60", value: a.name, "aria-label": "New name" });
  const msg = h("p", { class: "err-line", role: "alert", hidden: true });
  const save = h("button", { type: "submit", class: "btn p" }, "Save changes");
  const form = h("form", { class: "form" }, h("h2", { id: "ren-t" }, `Rename ${a.name}`), input, msg,
    h("div", { class: "btns" }, h("button", { type: "button", class: "btn", onclick: () => { dlg.close(); dlg.remove(); } }, "Cancel"), save));
  const dlg = h("dialog", { class: "cdlg", "aria-labelledby": "ren-t" }, form);
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    try { await api("/api/apps", { action: "update", id: a.id, name: input.value }); dlg.close(); dlg.remove(); data = null; await load(); render(); } catch (x) { msg.textContent = x.message; msg.hidden = false; input.focus(); }
  });
  document.body.append(dlg);
  dlg.showModal();
  input.select();
}

async function remove(a) {
  const ok = await UI.confirmDialog({ title: `Remove ${a.name}?`, text: a.repo ? "It leaves this list. The repo's folder and its tests are not touched." : "It leaves this list. Its tests, memory and map stay saved, and its schedules stop until you add it again.", verb: "Remove from list", keep: "Keep the app", danger: false });
  if (!ok) return;
  try { await api("/api/apps", { action: "remove", id: a.id }); toast(`${a.name} was removed from the list. Its files are kept.`, true); } catch (e) { toast(e.message); }
  data = null;
  await load();
  render();
}

/// #/apps/add?app=<id>&kind=<kind>: open the dialog over Your apps.
function route() {
  if (closing || document.querySelector("dialog.appdlg")) return;
  const q = new URLSearchParams((location.hash.split("?")[1] || ""));
  if (!data) { load().then(route); return; }
  addDialog({ app: q.get("app") || undefined, kind: q.get("kind") || undefined });
}

window.Apps = { bar, page, route, reload: () => { data = null; load(); } };
})();
