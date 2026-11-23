// fastpilot ui: settings (AI model, Limits & permissions, Secrets) and Device. Plain DOM, no framework.
// Values from the server are always set as text, never as HTML.
"use strict";

const $body = () => document.getElementById("body");
const $top = () => document.getElementById("top");

function h(tag, attrs, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === false || v === null || v === undefined) continue;
    if (k === "class") el.className = v;
    else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid === null || kid === undefined || kid === false) continue;
    el.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
  return el;
}
function icon(name, cls) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", "ic" + (cls ? " " + cls : ""));
  svg.setAttribute("aria-hidden", "true");
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", "#i-" + name);
  svg.append(use);
  return svg;
}
const code = (t) => h("code", {}, t);

let toastTimer;
function clearToast() {
  clearTimeout(toastTimer);
  document.querySelectorAll(".toast").forEach((t) => t.remove());
}

function toast(msg, ok) {
  document.querySelectorAll(".toast").forEach((t) => t.remove());
  const t = h("div", ok ? { class: "toast ok", role: "status" } : { class: "toast", role: "alert" }, msg);
  document.body.append(t);
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.remove(), 7000);
}

async function api(path, body) {
  const opts = body === undefined ? {} : { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) };
  let r;
  try {
    r = await fetch(path, opts);
  } catch (_) {
    // The browser's own "Failed to fetch": the fastpilot ui process is not running any more.
    throw new Error("fastpilot is not answering: type `fastpilot` in the Terminal to start it again");
  }
  // E3: the session ended (12 hours, forgotten, or the dashboard restarted): the page reloads and a
  // remembered browser signs in again by itself; any other one is told how to sign in.
  if (r.status === 401) {
    location.reload();
    return new Promise(() => {});
  }
  if (!r.ok) throw new Error((await r.text()).replace(/^fastpilot ui: /, "").trim() || r.statusText);
  return r.json();
}

// ---- state ----
let S = null; // { path, settings: [...], defaults }
let savedAt = null; // { page, at }: the page where something was just saved
let doctorCache = null;
const get = (key) => (S && S.settings ? S.settings.find((s) => s.key === key) : undefined);

async function loadSettings() {
  S = await api("/api/settings");
}
// The first screen (s6a): Chat, the first item of the menu.
const HOME = "#/agents";
function currentPage() {
  return (location.hash || HOME).split("/").slice(1, 3).join("/");
}

// Text typed into a list field whose save failed, put back so it can be corrected.
const typed = {};

async function save(key, value, attempt) {
  try {
    S = await api("/api/settings", { key, value });
    savedAt = { page: currentPage(), at: new Date() };
    delete typed[key];
    clearToast();
    render();
  } catch (e) {
    if (attempt !== undefined) typed[key] = attempt;
    toast("Not saved: " + e.message);
    render();
    const again = document.querySelector(`input[data-key="${key}"]`);
    if (again) again.focus();
  }
}

// ---- controls ----
function envNote(s) {
  return s.source === "env" ? h("div", { class: "env" }, `Set in the environment, ${s.env} (it wins over the file); change it in the MCP configuration.`) : null;
}
function opt(label, desc, control, s, extra) {
  return h("div", { class: "opt" }, h("div", {}, h("div", { class: "l" }, label), desc ? h("div", { class: "d" }, desc) : null, extra || null, s ? envNote(s) : null), control);
}
// Whether block_actions is in force: the built-in list unless emptied in the file (an empty
// environment variable does not empty it).
function blockingOn() {
  const b = get("block_actions");
  return !(b && b.source === "file" && Array.isArray(b.value) && !b.value.length);
}
// What a switch does now: allow_eval_script not set is off while block_actions is on (option C).
function effectiveOn(key) {
  const s = get(key);
  if (s.value !== null) return !!s.value;
  return key === "allow_eval_script" ? !blockingOn() : true;
}
function toggle(key, label) {
  const s = get(key);
  const on = effectiveOn(key);
  return h("button", {
    class: "tg", role: "switch", "aria-checked": String(on), "aria-label": label, disabled: s.source === "env",
    onclick: () => save(key, !on),
  });
}
// Built-in list of a setting (v0.3.2: block_actions, confirm_actions), in force while it is not set.
const builtinOf = (key) => (S && S.defaults && Array.isArray(S.defaults[key]) ? S.defaults[key] : null);

// A list with a built-in part: the words shown (built-in ones flagged) and what to save after an
// add or a removal. block_actions keeps `default` in the file (built-in words match whole words);
// confirm_actions writes its words out.
function withBuiltin(key, s) {
  const builtin = builtinOf(key);
  const value = Array.isArray(s.value) ? s.value : [];
  if (!builtin) return null;
  const unset = s.source === "default" || s.value === null;
  // D9 (M9): `!pay` allows one built-in word again and keeps every other built-in rule.
  const bangs = unset ? [] : value.filter((w) => w.startsWith("!"));
  const allowed = bangs.map((w) => w.slice(1).toLowerCase());
  const own = unset ? [] : value.filter((w) => w.toLowerCase() !== "default" && !w.startsWith("!"));
  const hasDefault = unset || value.some((w) => w.toLowerCase() === "default");
  const shownBuiltin = (hasDefault ? (key === "block_actions" || unset ? builtin : []) : []).filter((w) => !allowed.includes(w.toLowerCase()));
  const words = [...shownBuiltin.map((w) => ({ w, builtin: true })), ...own.map((w) => ({ w, builtin: false }))];
  const base = key === "block_actions" ? (hasDefault ? ["default", ...bangs, ...own] : own) : (unset ? [...builtin] : value);
  return {
    unset,
    words,
    // A built-in word allowed earlier is blocked again by dropping its `!`.
    add: (v) => (allowed.includes(v.toLowerCase()) ? base.filter((w) => w.toLowerCase() !== "!" + v.toLowerCase()) : [...base, v]),
    // block_actions: removing a built-in word allows only that word (`!word`); confirm_actions writes
    // its words out.
    remove: (it) => it.builtin
      ? (key === "block_actions" ? ["default", ...bangs, "!" + it.w, ...own] : [...shownBuiltin.filter((w) => w !== it.w), ...own])
      : (key === "block_actions" && hasDefault ? ["default", ...bangs, ...own.filter((w) => w !== it.w)] : value.filter((w) => w !== it.w)),
  };
}

function chips(key, opts) {
  const s = get(key);
  const bi = withBuiltin(key, s);
  const items = bi ? bi.words.map((x) => x.w) : (Array.isArray(s.value) ? s.value : []);
  const locked = s.source === "env";
  const input = h("input", { type: "text", placeholder: opts.placeholder, "aria-label": opts.addLabel, disabled: locked });
  const empty = !items.length && opts.empty ? h("span", { class: "chip none" }, opts.empty) : null;
  const preset = opts.preset && !items.includes(opts.preset[0]) && !locked
    ? h("button", { class: "chip preset", onclick: () => save(key, [...items, opts.preset[0]]) }, icon("plus", "sm"), opts.preset[1])
    : null;
  input.dataset.key = key;
  if (typed[key]) input.value = typed[key];
  const add = () => {
    const v = input.value.trim();
    if (!v) return;
    if (v.includes(",")) return toast("One item at a time, without commas.");
    if (items.some((x) => x.toLowerCase() === v.toLowerCase())) return toast(`"${v}" is already in the list.`);
    save(key, bi ? bi.add(v) : [...items, v], v);
  };
  input.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); add(); } });
  // A message about the last attempt goes away as soon as the user changes the text.
  input.addEventListener("input", () => { delete typed[key]; clearToast(); });
  const list = bi ? bi.words : items.map((w) => ({ w, builtin: false }));
  return h("div", { class: "chips" },
    empty,
    list.map((it) =>
      h("span", { class: "chip" + (opts.red ? " x" : "") + (it.builtin ? " dflt" : ""), title: it.builtin ? "built-in word" : null }, opts.show ? opts.show(it.w) : it.w,
        locked ? null : h("button", { class: "rm", "aria-label": `Remove ${it.w}`, onclick: () => save(key, bi ? bi.remove(it) : items.filter((x) => x !== it.w)) }, icon("close", "sm")))),
    preset,
    locked ? null : h("span", { class: "add" }, input, h("button", { onclick: add, "aria-label": opts.addLabel }, icon("plus", "sm"), "Add")),
    bi && !locked ? h("span", { class: "acts" },
      h("button", { class: "btn sm", disabled: !bi.unset && !items.length, onclick: () => save(key, []) }, "Empty it"),
      h("button", { class: "btn sm", disabled: bi.unset, onclick: () => save(key, null) }, "Back to built-in")) : null);
}
function select(key, choices, label) {
  const s = get(key);
  const cur = s.value === null ? "" : String(s.value);
  const list = [...choices];
  if (cur && !list.some(([v]) => v === cur)) list.push([cur, cur]);
  const el = h("select", { class: "sel", "aria-label": label, disabled: s.source === "env", onchange: (e) => save(key, e.target.value === "" ? null : e.target.value) },
    list.map(([v, t]) => { const o = h("option", { value: v }, t); if (v === cur) o.selected = true; return o; }));
  return el;
}
function savedNote() {
  if (!savedAt || savedAt.page !== currentPage()) return h("small", {}, "");
  const t = savedAt.at.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
  return h("small", {}, `✓ Saved · ${t}`);
}

// ---- pages ----
function settingsNav(cur) {
  const item = (id, label) => h("a", { href: `#/settings/${id}`, class: cur === id ? "on" : "", "aria-current": cur === id ? "page" : false }, label);
  return h("nav", { class: "snav", "aria-label": "Settings" }, item("model", "AI model"), item("limits", "Limits & permissions"), item("secrets", "Secrets"), item("security", "Security"), item("apps", "Your apps"),
    window.Welcome ? Welcome.fullMenuToggle() : null);
}

function topSettings() {
  UI.pageHeader({ icon: "settings", title: "Settings" });
}

const MODELS = [
  ["claude-haiku-5-5", "Claude Code · Haiku 5.5"],
  ["claude-sonnet-5-5", "Claude Code · Sonnet 5.5"],
  ["claude-opus-5-5", "Claude Code · Opus 5.5"],
  ["claude-fable-5-2", "Claude Code · Fable 5.2"],
];

// Who drives `fastpilot run`: Claude Code, or an OpenAI-compatible API (B2).
let provData = null, provBusy = false, provResult = null;
// What the user picked and typed but did not save yet: the page is redrawn (device check done,
// Save busy, a refused save) and must not throw it away. The API key is never kept here.
let provDraft = null;
const PROV_LABEL = { claude: "Claude Code (built in, no API key)", openai: "OpenAI", openrouter: "OpenRouter", groq: "Groq", ollama: "Ollama (local)", custom: "Other (OpenAI-compatible)" };

async function loadProvider() {
  try { provData = await api("/api/provider"); } catch (e) { provData = { error: e.message }; }
  if ((location.hash || "").startsWith("#/settings/model")) render();
}

async function provAction(body) {
  provBusy = true;
  render();
  try {
    const r = await api("/api/provider", body);
    provResult = { ok: true, text: r.message };
    if (body.action === "save") provDraft = null;
    if (body.action !== "test") toast(r.message, true);
  } catch (e) {
    provResult = { ok: false, text: e.message };
    if (body.action !== "test") toast(e.message);
  }
  provBusy = false;
  await loadProvider();
}

function providerGroup() {
  if (!provData) { loadProvider(); return h("div", { class: "grp" }, h("div", { class: "gh" }, icon("plug"), "who runs the tests"), h("div", { class: "empty small" }, "Reading…")); }
  if (provData.error) return UI.notice("error", "Cannot read the provider: " + provData.error);
  const preset0 = (n) => provData.presets.find((p) => p.name === n) || {};
  const d = provDraft || { provider: provData.provider, base_url: provData.base_url || preset0(provData.provider).base_url || "", model: provData.model || "" };
  const pick = h("select", { class: "sel", "aria-label": "Model provider for fastpilot run", disabled: provData.env.provider },
    provData.presets.map((p) => h("option", { value: p.name, selected: p.name === d.provider }, PROV_LABEL[p.name] || p.name)));
  const preset = (n) => provData.presets.find((p) => p.name === n) || {};
  const hosted = (n) => ["openai", "openrouter", "groq"].includes(n);
  const url = h("input", { type: "text", class: "mono", value: d.base_url, placeholder: "https://…/v1", "aria-label": "Base URL" });
  const urlHint = h("span", { class: "s" });
  // Only Claude Code was tried with fastpilot's demo; every other provider says so (v0.4).
  const untested = h("p", { class: "s prov-untested" }, h("span", { class: "ag-untested" }, "not tested yet"),
    " fastpilot has not been tried with this provider yet: run a few tests and check the results before you rely on them.");
  const model = h("input", { type: "text", class: "mono", value: d.model, placeholder: "for example gpt-4.1-mini, llama3.1", "aria-label": "Model id" });
  const keepDraft = () => { provDraft = { provider: pick.value, base_url: url.value, model: model.value }; };
  url.addEventListener("input", keepDraft);
  model.addEventListener("input", keepDraft);
  const k = provData.key || {};
  const key = h("input", { type: "password", class: "mono", autocomplete: "off", placeholder: k.has ? `saved in ${k.where} · type to replace it` : "sk-…", "aria-label": "API key" });
  const detail = h("div", { class: "prov-fields" },
    h("label", { class: "fld" }, h("span", { class: "l" }, "Base URL"), url, urlHint),
    h("label", { class: "fld" }, h("span", { class: "l" }, "Model"), model),
    h("label", { class: "fld" }, h("span", { class: "l" }, "API key"), key,
      h("span", { class: "s" }, provData.env.key ? "FASTPILOT_PROVIDER_API_KEY is set in the environment and is used first."
        : k.has ? `Saved in ${k.where}${k.origin ? `, only for ${k.origin}` : ""}. Never shown again; the key is only sent to the server it was saved for.`
        : `Saved in ${k.where || "the system key store"}, not in the settings file, and only sent to this server. A local Ollama needs no key.`)));
  // Changing provider changes the server: the preset's URL replaces the previous one (Design must-fix).
  let shownFor = d.provider;
  const sync = () => {
    detail.hidden = pick.value === "claude";
    untested.hidden = pick.value === "claude";
    const p = preset(pick.value);
    if (pick.value !== shownFor) {
      url.value = p.base_url || "";
      shownFor = pick.value;
    }
    url.readOnly = hosted(pick.value);
    urlHint.textContent = hosted(pick.value)
      ? `${PROV_LABEL[pick.value]} always uses this server. For another server choose "Other (OpenAI-compatible)".`
      : pick.value === "custom" ? "Required, for example https://llm.company.com/v1. https is required, except to this computer."
      : "Ollama's default on this computer; change it if Ollama uses another port.";
  };
  pick.addEventListener("change", () => { sync(); keepDraft(); });
  sync();
  const btns = h("div", { class: "btns" },
    h("button", { class: "btn p", disabled: provBusy || provData.env.provider, onclick: () => provAction({ action: "save", provider: pick.value, base_url: url.value, model: model.value, key: key.value }) }, "Save changes"),
    provData.provider !== "claude" ? h("button", { class: "btn", disabled: provBusy, onclick: () => provAction({ action: "test" }) }, provBusy ? "Testing…" : "Test the connection") : null,
    provData.provider !== "claude" && k.has ? h("button", { class: "btn danger", disabled: provBusy, onclick: async () => { if (await UI.confirmDialog({ title: "Delete the saved API key?", text: "The key is removed from this computer. Tests cannot use this provider until you enter a key again. Your other settings stay.", verb: "Delete key" })) provAction({ action: "delete-key" }); } }, "Delete key") : null);
  return h("div", { class: "grp" },
    h("div", { class: "gh" }, icon("plug"), "who runs the tests · fastpilot run"),
    provData.env.provider ? h("div", { class: "banner" }, icon("info"), h("span", {}, "The provider is set in the environment (FASTPILOT_PROVIDER…), which wins over this setting.")) : null,
    opt("Provider", "Claude Code uses your login. An OpenAI-compatible API is optional; it applies from the next run.", pick, null),
    h("div", { class: "prov" }, untested, detail, btns,
      provResult ? h("p", { class: "prov-result " + (provResult.ok ? "ok" : "bad"), role: "status" }, provResult.ok ? "✓ " : "✗ ", provResult.text) : null));
}

// S6: one line says what runs the tests (the signed-in Claude Code, found by itself); the models
// and other providers are under More options, which says how many differ from the defaults.
function modelUsing(run) {
  if (provData && !provData.error && provData.provider && provData.provider !== "claude") {
    return h("p", { class: "using", role: "status" }, `Using ${PROV_LABEL[provData.provider] || provData.provider}${provData.model ? " · " + provData.model : ""}`,
      h("span", { class: "s muted" }, " · change it under More options"));
  }
  if (!doctorCache) return h("p", { class: "using s muted", role: "status" }, "Looking for Claude Code…");
  if (run && run.status === "ok") return h("p", { class: "using ok", role: "status" }, "Using Claude Code ✓", h("span", { class: "s muted" }, ` · ${run.detail} · your own sign-in, no API key`));
  return UI.notice("warn", "Claude Code is not on this computer yet. fastpilot uses your own Claude Code sign-in, with no API key. ",
    h("a", { class: "lk", href: "https://claude.com/claude-code", target: "_blank", rel: "noopener" }, "Get Claude Code"), ", sign in once, then come back.");
}

function pageModel() {
  const run = doctorCache && doctorCache.checks.find((c) => c.area === "run");
  if (!provData) loadProvider();
  const status = !doctorCache ? h("span", { class: "tag" }, "Checking…")
    : run && run.status === "ok" ? h("span", { class: "tag" }, "✓ Installed")
    : h("span", { class: "tag warn" }, "Not installed");
  const changed = ["model_replay", "model_explore"].filter((k) => get(k) && get(k).value !== null).length + (provData && provData.provider && provData.provider !== "claude" ? 1 : 0);
  const replay = [["", `Built in · ${MODELS.find(([v]) => v === S.defaults.model_replay)?.[1] || S.defaults.model_replay}`], ...MODELS];
  const explore = [["", `Built in · ${MODELS.find(([v]) => v === S.defaults.model_explore)?.[1] || S.defaults.model_explore}`], ...MODELS];
  return h("section", { class: "panel" },
    h("h2", {}, "AI model", savedNote()),
    modelUsing(run),
    // Open by itself only when another provider is in use: its key and model are managed there.
    UI.moreOptions({ label: "More options", changed, open: !!(provData && provData.provider && provData.provider !== "claude") },
      h("p", { class: "sub" }, "fastpilot picks the models by itself: a fast one to replay recorded steps, a balanced one to explore. These only apply when fastpilot runs tests itself; from Claude Code, the model is chosen in that app."),
      h("div", { class: "grp" },
        h("div", { class: "gh" }, icon("model"), "ai model · who runs what"),
        opt("Replay (fast & cheap)", "Replays steps that are already recorded", select("model_replay", replay, "Replay model"), get("model_replay")),
        opt("Exploring / repairing", "Used when a replay fails, or for a new test", select("model_explore", explore, "Exploring model"), get("model_explore")),
        opt("Claude Code", run ? `${run.detail}` : "Uses your Claude Code login, no API key", status)),
      providerGroup()));
}

// The "Tests at once" list for Settings: the Live data says the current value, the most and the default.
function limitsTestsAtOnce() {
  if (lastLive && lastLive.limit) return limitPicker(lastLive);
  // Filled in place when the value arrives: redrawing the page would move the keyboard focus.
  const holder = h("span", { class: "s muted" }, "Reading…");
  api("/api/live").then((d) => { if (holder.isConnected && d.limit) holder.replaceWith(limitPicker(d)); }).catch(() => {});
  return holder;
}

// A2 (v0.7): what protects the user, in one statement worked out from the settings in force
// (never a claim the settings do not back: no "only your app's pages" while every domain is
// allowed), three example sentences that only make things stricter or show something, and every
// limit under More options. A sentence goes to Chat, where its card gates the change.
// "Never press" (a blocked word, refused even with confirm: true), not "ask me": nothing asks the
// person before a confirm-listed button (Security, #187 M1).
const LIMIT_EXAMPLES = ["Also block the word \"Refund\"", "Never press \"Log out\"", "Show me what is blocked"];
function limitsSummary() {
  const words = (key) => {
    const s = get(key);
    const dflt = (S.defaults && S.defaults[key]) || [];
    if (!s || s.value === null || s.value === undefined) return [...dflt];
    const v = Array.isArray(s.value) ? s.value : [];
    return v.flatMap((w) => (w === "default" ? dflt : [w]));
  };
  const blocked = words("block_actions");
  const confirm = words("confirm_actions");
  const quote = (w) => `"${w.charAt(0).toUpperCase()}${w.slice(1)}"`;
  // The best-known words first ("Pay", "Delete account", "Delete"), then the rest as listed.
  const pick = (list, first, n) => first.filter((w) => list.includes(w)).concat(list).filter((w, i, a) => a.indexOf(w) === i).slice(0, n);
  const steps = (get("max_steps") && get("max_steps").value) || S.defaults.max_steps || 30;
  const mins = (get("test_timeout") && get("test_timeout").value) || S.defaults.test_timeout || 30;
  const domains = (get("allowed_domains") && get("allowed_domains").value) || [];
  // A page script could press any button (the setting's own help), so "never pressed" does not hold then.
  const scripts = get("allow_eval_script") && get("allow_eval_script").value === true;
  const named = pick(blocked, ["pay", "delete account", "buy", "checkout"], 2).map(quote).join(" and ");
  const parts = [];
  if (blocked.length) parts.push(scripts ? `Test steps never press buttons like ${named}, but page scripts may run, and a script can press any button.` : `Buttons like ${named} are never pressed.`);
  if (confirm.length) parts.push(`Buttons like ${pick(confirm, ["delete", "remove"], 1).map(quote).join("")} are only pressed when the test step says it means it.`);
  if (domains.length) parts.push(`Tests only open ${domains.slice(0, 3).join(", ")}${domains.length > 3 ? " and more" : ""}.`);
  parts.push(`Every test fastpilot runs (Run, schedules) stops after ${mins} minutes and ${steps} AI steps.`);
  const head = blocked.length && !scripts
    ? h("div", { class: "protect ok", role: "status" }, h("span", { class: "protect-mark", "aria-hidden": "true" }, "✓"), h("div", {}, h("b", {}, "You are protected"), h("p", {}, parts.join(" "))))
    : h("div", { class: "protect warn", role: "status" }, h("span", { class: "protect-mark", "aria-hidden": "true" }, "!"), h("div", {}, h("b", {}, blocked.length ? "Less protected: page scripts may run" : "Less protected: no button is blocked"), h("p", {}, parts.join(" "), blocked.length ? " Turn \"Run any JavaScript\" off under More options." : " Turn the built-in list back on under More options.")));
  const say = (t) => () => openChat(t);
  return [head,
    h("section", { class: "grp limit-say", "aria-label": "Change something in Chat" },
      h("div", { class: "gh" }, "want to change something? just say it in chat"),
      h("div", { class: "prop-body" },
        LIMIT_EXAMPLES.map((t) => h("button", { class: "say-row", onclick: say(t) }, h("span", {}, t), h("span", { class: "arrow", "aria-hidden": "true" }, "→"))),
        h("p", { class: "s muted" }, "One click sends it to Chat. Stricter changes apply at once, with Undo; anything that loosens a rule shows a red card and needs your own words."))) ];
}

function pageLimits() {
  const confirmShow = (w) => (w === "default" ? "default (delete, pay, buy, …)" : w);
  const emptied = (key) => { const s = get(key); return s && s.source !== "default" && Array.isArray(s.value) && !s.value.length; };
  return h("section", { class: "panel" },
    h("h2", {}, "Limits & permissions", savedNote()),
    limitsSummary(),
    UI.moreOptions({ label: "More options", changed: S.settings.filter((s) => ["allowed_domains", "block_actions", "confirm_actions", "allow_app_install", "allow_app_reset", "allow_physical_devices", "allow_device_settings", "allow_eval_script", "allow_takeover", "upload_dirs", "state_expire_days", "max_steps", "test_timeout"].includes(s.key) && s.source === "file").length },
    h("p", { class: "s muted" }, "See and change every limit by hand."),
    h("p", { class: "sub" }, "These rules apply to every test: from Chat, Claude Code, Cursor, the terminal and schedules."),
    h("div", { class: "banner" }, icon("info"), h("span", {}, "Changes apply from the next run. A session that is running keeps the rules it started with. Through chat, a looser rule needs your own words and your typed confirmation on its red card. Saved in ", code(S.path), ".")),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("globe"), "domains that may be opened"),
      opt("Only allow these domains", "Empty = every domain is allowed. Links, redirects and pop-ups to other domains are blocked.",
        chips("allowed_domains", { placeholder: "for example shop.com", addLabel: "Add a domain", empty: "Every domain is allowed" }), get("allowed_domains"))),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("risk"), "risky actions"),
      opt("Buttons that are always blocked", "Also with confirm: true. Dashed = built-in word.",
        chips("block_actions", { placeholder: "for example Log out", addLabel: "Add a blocked word", red: true, empty: emptied("block_actions") ? "Emptied: nothing is blocked" : "None yet" }), get("block_actions"),
        h("div", { class: "d" }, "Built-in words match whole words (\"pay\" matches \"Pay now\", not \"Paypal\"). Your own words match when the button text ", h("b", {}, "contains"), " the word: \"Log out\" also blocks \"Log out everywhere\". While there is a word here, free JavaScript (evalScript) is off too, unless you turn on \"Run any JavaScript\" below.")),
      opt("Buttons that need confirmation", "The AI must write confirm: true. Whole-word match. Built in: hapus, delete, remove.",
        chips("confirm_actions", { placeholder: "for example Archive", addLabel: "Add a confirmation word", show: confirmShow, empty: emptied("confirm_actions") ? "Emptied: no confirmation" : "None yet" }), get("confirm_actions"))),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("device"), "devices & apps"),
      opt("May install / uninstall apps", "Install or remove apps on phones, emulators or simulators", toggle("allow_app_install", "May install / uninstall apps"), get("allow_app_install")),
      opt("May clear app data (reset)", "Clears the app's data and login, for tests that start from scratch", toggle("allow_app_reset", "May clear app data"), get("allow_app_reset")),
      opt("May use physical phones", "When off, only emulators and simulators", toggle("allow_physical_devices", "May use physical phones"), get("allow_physical_devices")),
      opt("May change phone settings (network, location, app permissions, language)", "For example: offline mode, fake GPS, camera permission, phone language", toggle("allow_device_settings", "May change phone settings"), get("allow_device_settings"))),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("lock"), "data & security"),
      opt("Run any JavaScript on the page", "Turn off to stop the AI from changing the page outside the test steps", toggle("allow_eval_script", "Run any JavaScript"), get("allow_eval_script"),
        get("allow_eval_script").value === null && blockingOn()
          ? h("div", { class: "d warn" }, "Off because the block list is on (a script could press a blocked button); turn on to allow it.")
          : null),
      opt("May take over sessions (Take over in Live)", "Turn off when your agent has shell access: input during a takeover is not checked by Limits. Applies to fastpilot processes that start after this.", toggle("allow_takeover", "May take over sessions"), get("allow_takeover")),
      opt("Folders files may be uploaded from", "Files a test uploads must come from these folders.",
        chips("upload_dirs", { placeholder: "for example ~/fixtures", addLabel: "Add a folder", empty: "Any folder" }), get("upload_dirs")),
      opt("How long a saved login lasts", "Saved logins older than this are not used again",
        select("state_expire_days", [["", "30 days (built in)"], ["1", "1 day"], ["7", "7 days"], ["14", "14 days"], ["90", "90 days"]], "How long a saved login lasts"), get("state_expire_days")),
      opt("Hide secrets in all output", "Secret values show as *** in output, reports and recordings", h("span", { class: "fixed" }, "Always on"))),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("gauge"), "usage limits · fastpilot run"),
      opt("Most AI steps per test", "The test stops and fails when it goes over",
        select("max_steps", [["", "30 steps (built in)"], ["10", "10 steps"], ["20", "20 steps"], ["40", "40 steps"], ["60", "60 steps"], ["100", "100 steps"]], "Most steps"), get("max_steps")),
      // UXA5: "Tests at once" lives here (advanced), with its default marked; not on the Live header.
      opt("Tests at once", "How many tests may run at the same time on this computer, for all agents together. One device always runs one test.", limitsTestsAtOnce()),
      opt("Longest time per test", "The test stops and fails when it goes over",
        select("test_timeout", [["", "30 minutes (built in)"], ["5", "5 minutes"], ["10", "10 minutes"], ["15", "15 minutes"], ["60", "60 minutes"]], "Longest time"), get("test_timeout")))));
}

// E2: secret names, and for the current app whether each has a value (never the value).
let secData2 = null;
async function loadSecrets() {
  try { secData2 = await api("/api/secrets"); } catch (e) { secData2 = { error: e.message, names: [] }; }
  if (location.hash.startsWith("#/settings/secrets")) render();
}

function pageSecret() {
  if (!secData2) { loadSecrets(); return h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" }), h("b", {}, "Reading secrets…")); }
  if (secData2.error) return UI.notice("error", "Could not read the secrets: " + secData2.error);
  const reload = () => { secData2 = null; render(); };
  const app = secData2.app;
  const row = (r) => h("tr", { "data-name": r.name },
    h("th", { scope: "row" }, r.name),
    h("td", {}, r.env ? "From the environment" : r.set ? `Saved on this ${secData2.kept_in === "this Mac's Keychain" ? "Mac" : "computer"}` : "Not set yet: you will be asked when a test needs it"),
    h("td", {}, app ? h("span", { class: "btns tight" },
      h("button", { type: "button", class: "btn sm" + (r.set ? "" : " p"), onclick: () => UI.passwordAsk({ name: r.name, label: r.set ? `Replace ${r.name}` : `Set ${r.name}`, onSaved: () => { toast(`${r.name} is saved for ${app.name}.`, true); reload(); } }) }, r.set ? "Replace" : "Set value"),
      r.set ? h("button", { type: "button", class: "btn sm", onclick: async () => {
        if (!(await UI.confirmDialog({ title: `Remove ${r.name}?`, text: `Its value is deleted from this Mac's key store for ${app.name}. Tests that use it stop before they type anything, until you set it again.`, verb: "Remove value", danger: false }))) return;
        try { await api("/api/secrets", { action: "remove", name: r.name }); toast(`${r.name} was removed.`, true); } catch (e) { toast(e.message); }
        reload();
      } }, "Remove") : null) : null));
  return h("section", { class: "panel" },
    h("h2", {}, "Secrets", savedNote()),
    app
      ? h("p", { class: "sub" }, `fastpilot asks for a password the first time a test of ${app.name} needs it, then remembers it on this ${secData2.kept_in === "this Mac's Keychain" ? "Mac" : "computer"}. It is never shown again, and it is typed only on this app's own website.`)
      : h("p", { class: "sub" }, "Add or choose an app first: passwords belong to an app."),
    secData2.names.length
      ? h("div", { class: "tbl-wrap" }, h("table", { class: "tbl" }, h("caption", { class: "sr" }, "Secrets"),
        h("thead", {}, h("tr", {}, ["Name", "Value", ""].map((t) => h("th", { scope: "col" }, t)))), h("tbody", {}, secData2.names.map(row))))
      : UI.empty({ icon: "key", title: "Nothing saved yet", text: "You will be asked when a test needs one." }),
    UI.moreOptions({ label: "Use a secret from the environment instead" },
      opt("Secret name", "For CI and scripts: list the name here, and give the value in the environment fastpilot starts in. Or add a name and press Set value.",
        chips("vars", { placeholder: "for example DEMO_PASS", addLabel: "Add a secret name", empty: "None yet" }), get("vars")),
      UI.moreOptions({ label: "Show the command" }, h("p", { class: "s" }, code("export FASTPILOT_VARS=DEMO_PASS DEMO_PASS=…"), " before starting fastpilot or Claude Code."))));
}

// The settings file does not parse: say what and where, and offer nothing to change (the server
// refuses to save until it is fixed; fastpilot itself stops with the same error).
function pageBroken() {
  const reload = async () => {
    try { await loadSettings(); } catch (e) { toast("Could not reload: " + e.message); }
    render();
    // Keyboard focus stays on the page (the button it was on is gone): the page title.
    document.querySelector(".top h1")?.focus();
  };
  return h("section", { class: "panel" },
    h("h2", {}, "Settings"),
    h("div", { class: "err big-err", role: "alert" },
      h("b", {}, "The settings file is damaged. "),
      "Tests will not run, and no change can be saved, until this file is fixed or deleted.",
      h("div", { class: "path" }, code(S.path)),
      h("pre", { class: "errtext" }, S.error)),
    h("div", { class: "btns" }, h("button", { class: "btn p", onclick: reload }, icon("refresh", "sm"), "Reload after fixing it")),
    h("p", { class: "muted" }, "Same as the message ", code("fastpilot config list"), ". Deleting the file puts everything back to built-in."));
}

function pageSettings(sub) {
  if (sub === "secret") sub = "secrets"; // links made before v0.7
  topSettings();
  if (S.error) {
    $body().replaceChildren(h("div", { class: "set" }, settingsNav(["model", "secrets", "security", "apps"].includes(sub) ? sub : "limits"), pageBroken()));
    return;
  }
  const page = sub === "model" ? pageModel() : sub === "secrets" ? pageSecret() : sub === "security" ? pageSecurity() : sub === "apps" ? Apps.page() : pageLimits();
  $body().replaceChildren(h("div", { class: "set" }, settingsNav(["model", "secrets", "security", "apps"].includes(sub) ? sub : "limits"), page));
  if (sub === "model" && !doctorCache) loadDoctor(false);
}

// ---- Settings › Security: remembered browsers (E3) ----
const REMEMBER_KEY = "fastpilot.browser";
let secData = null;
const day = (ms) => new Date(ms).toLocaleDateString("en-GB", { day: "numeric", month: "short" });

async function loadBrowsers() {
  try { secData = await api("/api/browsers"); } catch (e) { secData = { error: e.message, browsers: [] }; }
  if (location.hash.startsWith("#/settings/security")) render();
}

async function forgetBrowsers(b) {
  const all = !b;
  const self = b && b.current;
  const n = secData.browsers.length;
  const ok = await UI.confirmDialog(all
    ? { title: `Forget all ${n === 1 ? "1 browser" : `${n} browsers`}?`, text: "Every browser will need a new link from the Terminal, including this one if it is remembered.", verb: "Forget all" }
    : self
      ? { title: "Forget this browser?", text: "You are using it now. You will be signed out right away and need a new link from the Terminal.", verb: "Forget and sign out" }
      : { title: `Forget ${b.label}?`, text: "It will need a new link from the Terminal the next time. Your tests and settings are not affected.", verb: `Forget ${b.label.split(" on ")[0]}` });
  if (!ok) return;
  try {
    const r = await api("/api/browsers", all ? { action: "forget_all" } : { action: "forget", id: b.id });
    if (r.signed_out) {
      localStorage.removeItem(REMEMBER_KEY);
      location.reload();
      return;
    }
    toast(all ? "All browsers are forgotten." : `${b.label} is forgotten.`, true);
  } catch (e) { toast(e.message); }
  loadBrowsers();
}

function pageSecurity() {
  if (!secData) { loadBrowsers(); return h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" }), h("b", {}, "Reading the browsers…")); }
  if (secData.error) return UI.notice("error", "Could not read the remembered browsers: " + secData.error);
  const list = secData.browsers;
  const table = list.length
    ? h("div", { class: "tbl-wrap" }, h("table", { class: "tbl" }, h("caption", { class: "sr" }, "Remembered browsers"),
      h("thead", {}, h("tr", {}, ["Browser", "Remembered", "Last used", "Stays signed in until", ""].map((t) => h("th", { scope: "col" }, t)))),
      h("tbody", {}, list.map((b) => h("tr", { "data-id": b.id },
        h("th", { scope: "row" }, b.label, b.current ? h("span", { class: "tag" }, " This browser") : null),
        h("td", {}, day(b.created_ms)), h("td", {}, day(b.last_ms)), h("td", {}, day(b.until_ms)),
        h("td", {}, h("button", { class: "btn sm", onclick: () => forgetBrowsers(b) }, "Forget")))))))
    : UI.empty({ icon: "lock", title: "No browser is remembered", text: "When you open the dashboard, choose \"Remember this browser\" to open it from a bookmark for 30 days, without a new link." });
  return h("div", { class: "page" },
    h("h2", {}, "Security", savedNote()),
    h("section", { class: "grp" }, h("div", { class: "gh" }, icon("lock"), `browsers that can open the dashboard · only on this Mac · 127.0.0.1:${secData.port}`),
      table,
      list.length ? h("div", { class: "btns" }, h("button", { class: "btn danger", onclick: () => forgetBrowsers(null) }, "Forget all browsers"),
        h("span", { class: "s muted" }, "The next visit from each needs a new link from the Terminal.")) : null),
    UI.notice("info", "The dashboard only answers this Mac, and only a link or a browser you allowed can open it. A remembered browser stays signed in for 30 days after its last use, and never longer than 90 days."));
}

/// The first-open question (asked once; "Not now" asks again at most once a week).
async function askRemember() {
  let d;
  try { d = await api("/api/browsers"); } catch (_) { return; }
  if (d.current) return;
  const key = localStorage.getItem(REMEMBER_KEY);
  if (key) {
    // Remembered before, opened with a link now: sign in with the key, no question.
    try { await fetch("/api/unlock", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ key }) }); } catch (_) { /* the link's session stays */ }
    return;
  }
  const later = Number(localStorage.getItem("fastpilot.notnow") || 0);
  if (Date.now() - later < 7 * 86400000) return;
  const bar = h("section", { class: "notice info remember", role: "region", "aria-label": "Remember this browser" },
    h("span", {}, h("b", {}, "Note: "), "Remember this browser? Next time you can open the dashboard from a bookmark, without a new link. It stays signed in for 30 days, only on this Mac, and you can forget it any time in Settings › Security."),
    h("span", { class: "btns tight" },
      h("button", { class: "btn p", onclick: async () => {
        try {
          const r = await api("/api/browsers", { action: "remember" });
          if (r.key) localStorage.setItem(REMEMBER_KEY, r.key);
          bar.remove();
          toast(`Done. Bookmark 127.0.0.1:${location.port} and open it any time until ${day(r.until_ms)}. Manage it in Settings › Security.`, true);
          secData = null;
        } catch (e) { toast("Could not remember this browser: " + e.message); }
      } }, "Remember this browser"),
      h("button", { class: "btn", onclick: () => { localStorage.setItem("fastpilot.notnow", String(Date.now())); bar.remove(); } }, "Not now")));
  document.getElementById("main").prepend(bar);
}

// ---- device ----
// U1 (DC3): a hint that is a terminal command stays behind "Show the command".
const COMMAND = /`|\b(sudo|brew|xcode-select|fastpilot (setup|config|ui))\b|build\.sh|--remote-debugging/;
const AREAS = { web: "web · chrome", android: "android", ios: "ios", run: "fastpilot run · claude code" };
let doctorBusy = false;

async function loadDoctor(rerender = true) {
  if (doctorBusy) return;
  doctorBusy = true;
  if (rerender && location.hash.startsWith("#/device")) pageDevice();
  try {
    doctorCache = await api("/api/doctor");
    doctorCache.at = new Date();
  } catch (e) {
    doctorCache = { error: e.message, checks: [] };
  }
  doctorBusy = false;
  render();
}

function pageDevice() {
  const top = [icon("device"), h("h1", { tabindex: "-1" }, "Device")];
  if (doctorCache && !doctorCache.error) {
    const ok = doctorCache.checks.filter((c) => c.status === "ok").length;
    const bad = doctorCache.checks.length - ok;
    top.push(h("span", { class: "pill" + (bad ? " grey" : "") }, `● ${ok} ready · ${bad} need attention`));
  }
  $top().replaceChildren(...top);
  const header = h("div", { class: "gh" }, icon("device"), "device · check results",
    h("button", { class: "btn sm", onclick: () => loadDoctor(), disabled: doctorBusy }, icon("refresh", "sm"), doctorBusy ? "Checking…" : "Check again"));
  let content;
  if (doctorBusy || !doctorCache) {
    content = h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Checking" }), h("b", {}, "Checking devices…"),
      h("span", {}, "Chrome, Android, Xcode and Claude Code. Usually a few seconds, at most about 30 seconds."));
  } else if (doctorCache.error) {
    content = h("div", { class: "err", role: "alert" }, "Could not check the devices: " + doctorCache.error);
  } else {
    const phys = get("allow_physical_devices");
    const physOff = phys && phys.value === false;
    content = Object.keys(AREAS).filter((a) => doctorCache.checks.some((c) => c.area === a)).map((a) => [
      h("div", { class: "area" }, AREAS[a]),
      doctorCache.checks.filter((c) => c.area === a).map((c) =>
        h("div", { class: "row" },
          h("div", {},
            h("div", { class: "t" }, h("span", { class: "dot " + c.status }), c.name),
            h("div", { class: "s" }, c.detail),
            c.hint ? (COMMAND.test(c.hint) ? UI.moreOptions({ label: "Show the command" }, h("div", { class: "hint" }, c.hint)) : h("div", { class: "hint" }, "→ ", c.hint)) : null,
            null),
          h("span", { class: "tag" + (c.status === "ok" ? "" : " " + c.status) }, c.status === "ok" ? "Ready" : c.status === "warn" ? "Attention" : "Not ready"))),
      a === "android" && physOff ? h("div", { class: "row" }, h("div", { class: "s" }, "Phones plugged in stay off until you allow them in Limits, so only emulators are used. ", h("a", { class: "lk", href: "#/settings/limits" }, "Change it in Limits"))) : null,
    ]);
  }
  $body().replaceChildren(h("div", { class: "dev" },
    PAGES.deviceFix ? PAGES.deviceFix() : null,
    h("div", { class: "grp" }, header, content),
    sayIt(["Start the Android emulator", "Set up iPhone testing"]),
    h("p", { class: "muted" }, "The AI model and secrets are in Settings: ", h("a", { class: "lk", href: "#/settings/model" }, "AI model"), " · ", h("a", { class: "lk", href: "#/settings/secrets" }, "Secrets"), ".")));
}

// ---- live (C7a) ----
const PLATFORM = { web: "chrome · fastpilot's own", android: "emulator / HP", ios: "simulator / iPhone" };
const KIND = { mcp: "Claude Code / MCP", daemon: "CLI", run: "fastpilot run" };
let liveTimer = null;
let liveSel = null; // "pid:id"
const liveCards = new Map(); // key -> { el, line, status, more }
let stepsSig = "";

function stopLive() {
  clearTimeout(liveTimer);
  liveTimer = null;
  // Ending the streams ends the screencasts on the server.
  stopWatching();
  liveCards.clear();
  stepsSig = "";
  document.getElementById("live-root")?.remove();
  closeZoom(false);
}

function stepLine(steps) {
  const pick = (st) => steps.findIndex((s) => s.state === st);
  const running = pick("running"), stopped = steps.findIndex((s) => s.state === "blocked" || s.state === "failed");
  const at = running >= 0 ? running : stopped >= 0 ? stopped : steps.map((s) => s.state).lastIndexOf("done");
  if (at < 0) return steps.length ? `0/${steps.length} steps` : "No steps yet";
  return `Step ${at + 1}/${steps.length} · ${steps[at].label}`;
}

function sessionStatus(s) {
  if (s.taken_over_here) return { text: "Taken over by you · the AI's step waits", cls: "warn" };
  if (s.taken_over_ms) return { text: "Taken over on another page · the AI's step waits", cls: "warn" };
  const f = s.flow;
  if (!f) return { text: "Ready · waiting for a step", cls: "" };
  if (f.steps.some((x) => x.state === "blocked")) return { text: "Blocked by Limits", cls: "bad" };
  if (f.steps.some((x) => x.state === "failed")) return { text: "Failed", cls: "bad" };
  if (!f.finished) return { text: "Running", cls: "run" };
  return { text: `Done · ${f.steps.filter((x) => x.state === "done").length}/${f.steps.length} passed`, cls: "ok" };
}

// The engine compares in lower case; show the word as the user wrote it in Limits.
function asWritten(key, w) {
  const list = (get(key) && Array.isArray(get(key).value)) ? get(key).value : [];
  return list.find((x) => x.toLowerCase() === w.toLowerCase()) || w;
}

// Why a guard rail stopped a step, in a few words, from the engine's message.
function blockReason(note) {
  const n = note || "";
  let m = /contains "([^"]+)" \((the built-in blocked list|your blocked words)\)/.exec(n);
  if (m) return `the text contains "${asWritten("block_actions", m[1])}"${m[2] === "the built-in blocked list" ? " (built-in list)" : ""}`;
  m = /risky action \("([^"]+)"/.exec(n);
  if (m) return `needs confirm: true ("${asWritten("confirm_actions", m[1])}")`;
  if (/allowed_domains|ALLOWED_DOMAINS|not in the allowed/i.test(n)) return "domain not in the list";
  m = /\((allow_[a-z_]+) = false\)/.exec(n);
  if (m) return `turned off in Limits (${m[1]})`;
  if (/upload_dirs/.test(n)) return "file outside the upload folders";
  return "a rule in Limits";
}

// Read the MJPEG stream with fetch and show each JPEG as a blob URL: works in every browser,
// shows a static page's single frame, and stops (abort) when the view goes away.
async function watch(img, url, onFirst, onError) {
  const ctl = new AbortController();
  img._stop = () => ctl.abort();
  let shown = null;
  try {
    const r = await fetch(url, { signal: ctl.signal });
    if (!r.ok) throw new Error((await r.text()).replace(/^fastpilot ui: /, "").trim());
    const reader = r.body.getReader();
    let buf = new Uint8Array(0);
    const find = (hay, needle, from) => {
      outer: for (let i = from; i <= hay.length - needle.length; i++) {
        for (let j = 0; j < needle.length; j++) if (hay[i + j] !== needle[j]) continue outer;
        return i;
      }
      return -1;
    };
    const sep = new TextEncoder().encode("\r\n\r\n");
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      const merged = new Uint8Array(buf.length + value.length);
      merged.set(buf); merged.set(value, buf.length); buf = merged;
      for (;;) {
        const at = find(buf, sep, 0);
        if (at < 0) break;
        const head = new TextDecoder().decode(buf.slice(0, at));
        const m = /Content-Length:\s*(\d+)/i.exec(head);
        if (!m) { buf = buf.slice(at + 4); continue; }
        const len = Number(m[1]);
        if (buf.length < at + 4 + len) break;
        const jpeg = buf.slice(at + 4, at + 4 + len);
        buf = buf.slice(at + 4 + len);
        const next = URL.createObjectURL(new Blob([jpeg], { type: "image/jpeg" }));
        img.src = next;
        if (shown) URL.revokeObjectURL(shown); else onFirst();
        shown = next;
      }
    }
    if (!shown) throw new Error("no picture from this session");
  } catch (e) {
    if (e.name !== "AbortError") onError(e.message || String(e));
  }
}

function stopWatching(root) {
  (root || document).querySelectorAll("img.screen").forEach((i) => { if (i._stop) i._stop(); i._stop = null; });
}

function screenEl(s, big) {
  if (!s.watchable) {
    return h("div", { class: "screen-box none" }, h("span", {}, s.platform === "web"
      ? "Your own Chrome (attached) is not shown here."
      : `The ${s.platform} screen cannot be shown: this fastpilot process is an older version. Its steps still show below.`));
  }
  // Android/iOS frames are screenshots from the device (the server caps them at 3 per second).
  const fps = big ? (s.platform === "web" ? 8 : 3) : (s.platform === "web" ? 2 : 1), w = big ? 1280 : 480;
  const img = h("img", { class: "screen", alt: `Screen of session ${s.id}`, draggable: "false" });
  const status = h("div", { class: "screen-state" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" }), h("span", {}, "Connecting to the screen…"));
  const box = h("div", { class: "screen-box" }, img, status);
  const start = () => watch(img, `/api/live/frames?pid=${s.pid}&session=${encodeURIComponent(s.id)}&fps=${fps}&w=${w}`, () => status.remove(), (why) => {
    status.replaceChildren(h("span", { class: "bad" }, "The screen cannot be shown: " + why),
      h("button", { class: "btn sm", onclick: () => { status.replaceChildren(h("div", { class: "spin" }), h("span", {}, "Connecting to the screen…")); start(); } }, "Try again"));
    if (!box.contains(status)) box.append(status);
  });
  start();
  return box;
}

let lastLive = { sessions: [] };

function selectSession(key) {
  liveSel = key;
  renderLive(lastLive);
}

function card(s) {
  const key = `${s.pid}:${s.id}`;
  const line = h("div", { class: "l" });
  const status = h("div", { class: "d" });
  const zoom = s.watchable
    ? h("button", { class: "btn sm", "data-fk": `zoom:${key}`, "aria-label": `Enlarge the screen of session ${s.id}`, onclick: (e) => openZoom(s, e.currentTarget) }, icon("expand", "sm"), "Enlarge")
    : null;
  const take = s.controllable
    ? h("button", { class: "btn sm", "data-fk": `take:${key}`, onclick: (e) => toggleTakeover(key, e.currentTarget) })
    : null;
  const more = zoom || take ? h("div", { class: "btns tight" }, zoom, take) : null;
  const el = h("article", { class: "lcard", tabindex: "0", "data-fk": `card:${key}`, "aria-label": `Session ${s.id}, ${s.platform}. Enter to see its steps.`,
    onclick: (e) => { if (!e.target.closest("button")) selectSession(key); } },
    h("div", { class: "lh" }, h("span", { class: "pf " + s.platform }, s.platform), h("span", { class: "muted mono" }, PLATFORM[s.platform] || s.platform), h("span", { class: "chip" }, s.id)),
    s.agent_name ? h("div", { class: "l-agent" }, h("b", {}, s.agent_name), s.model_label ? h("span", { class: "s" }, ` · ${s.model_label}`) : null,
      s.model_tested === false ? h("span", { class: "ag-untested" }, "not tested yet") : null) : null,
    screenEl(s, false),
    h("div", { class: "card-text" }, h("div", {}, line, status), more));
  el.addEventListener("keydown", (e) => {
    if (e.target !== el) return;
    if (e.key === "Enter" || e.key === " ") { e.preventDefault(); selectSession(key); }
  });
  return { el, line, status, take };
}

// Only text and classes change on each poll: the buttons stay, so keyboard focus stays.
function fillCard(c, s, selected) {
  const st = sessionStatus(s);
  const steps = s.flow ? s.flow.steps : [];
  const lineText = stepLine(steps);
  if (c.line.textContent !== lineText) c.line.textContent = lineText;
  const statusText = st.text + (s.kind ? ` · ${KIND[s.kind] || s.kind}` : "");
  if (c.status.textContent !== statusText) c.status.textContent = statusText;
  c.status.className = "d " + st.cls;
  if (c.take) setTakeLabel(c.take, s);
  c.el.classList.toggle("sel", selected);
  if (selected) c.el.setAttribute("aria-current", "true"); else c.el.removeAttribute("aria-current");
}

function stepsPanel(s) {
  const steps = s.flow ? s.flow.steps : [];
  const limits = (i, change) => h("a", { class: "lk bad", href: "#/settings/limits", "data-fk": `limits:${i}` }, change ? "Change it in Limits →" : "See Limits");
  const items = steps.map((x, i) => {
    const n = `${i + 1} · `;
    if (x.state === "running") return h("li", { class: "now" }, h("span", { class: "i" }, icon("play", "sm")), h("span", {}, n, x.label), h("span", { class: "r" }, "running"));
    if (x.state === "done") return h("li", {}, h("span", { class: "i ok", "aria-label": "done" }, "✓"), h("span", {}, n, x.label));
    if (x.state === "failed") return h("li", { class: "blk" }, h("span", { class: "i", "aria-label": "failed" }, "✗"), h("span", {}, n, x.label), h("span", { class: "r" }, x.note || "failed"));
    if (x.state === "blocked") {
      // block_actions: the button, the word and where it comes from (v0.3.2 design).
      const b = /"([^"]*)"[^"]* contains "([^"]+)" \((the built-in blocked list|your blocked words)\)/.exec(x.note || "");
      const built = b && b[3] === "the built-in blocked list";
      const why = b
        ? h("span", { class: "r" }, h("b", {}, "Blocked by Limits. "), `The button text "${b[1]}" contains `, code(built ? b[2] : asWritten("block_actions", b[2])), built ? " (built-in list)" : "", ". The page was not pressed. ", limits(i, true))
        : h("span", { class: "r" }, `blocked: ${blockReason(x.note)} · `, limits(i));
      return h("li", { class: "blk" }, h("span", { class: "i", "aria-label": "blocked" }, icon("lock", "sm")), h("span", {}, n, x.label), why);
    }
    if (x.state === "skipped") return h("li", { class: "muted" }, h("span", { class: "i" }, "–"), h("span", {}, n, x.label, " · skipped"));
    return h("li", { class: x.will_block ? "warnline" : "" }, h("span", { class: "i" }, x.will_block ? icon("lock", "sm") : "○"), h("span", {}, n, x.label),
      x.will_block ? h("span", { class: "r" }, x.will_block, " · ", limits(i)) : null);
  });
  return h("div", { class: "grp steps-grp" },
    h("h2", { class: "gh" }, icon("tests"), `step history · ${s.id} (${s.platform} · the selected session)`),
    items.length ? h("ul", { class: "steps", "aria-live": "polite" }, items) : h("div", { class: "empty small" }, "This session has not run a step yet."));
}

function liveEmpty() {
  return h("div", { class: "grp" }, h("div", { class: "gh" }, "live · empty"),
    h("div", { class: "empty" }, icon("live", "big"), h("b", {}, "No test is running yet"),
      h("span", {}, "Start one from Chat and watch it here."),
      h("button", { type: "button", class: "btn p", onclick: () => openChat() }, icon("chat", "sm"), "Make a test in Chat")));
}

// ---- Take over (C7b) ----
// The user steers a session from here; its automatic steps wait until Give back.
function liveSession(key) {
  return (lastLive.sessions || []).find((x) => `${x.pid}:${x.id}` === key);
}

function setTakeLabel(btn, s) {
  const mine = !!s.taken_over_here, elsewhere = !!s.taken_over_ms && !mine, off = s.takeover_allowed === false && !mine;
  const label = mine ? "Give back to the AI" : elsewhere ? "Taken over on another page" : off ? "Take over is turned off" : "Take over";
  if (btn.dataset.label !== label) {
    btn.dataset.label = label;
    btn.replaceChildren(icon(mine ? "play" : "cursor", "sm"), label);
    btn.classList.toggle("p", mine);
    btn.disabled = elsewhere || off;
    btn.title = off ? "Turned off in Settings › Limits (allow_takeover)" : "";
    btn.setAttribute("aria-label", mine ? `Give session ${s.id} back to the AI` : elsewhere ? `Session ${s.id} is taken over on another page` : off ? `Take over is turned off in Limits` : `Take over session ${s.id}: the AI's steps pause`);
  }
}

async function control(s, op, extra) {
  try {
    await api("/api/live/control", Object.assign({ pid: s.pid, session: s.id, op }, extra || {}));
    return true;
  } catch (e) {
    toast((op === "takeover" ? "Cannot take over: " : op === "release" ? "Cannot give back: " : "Input not sent: ") + e.message);
    return false;
  }
}

async function refreshLive() {
  try { renderLive(await api("/api/live")); } catch (_) { /* the next poll shows it */ }
}

async function toggleTakeover(key, opener) {
  const s = liveSession(key);
  if (!s) return;
  if (s.taken_over_here) {
    if (await control(s, "release")) await refreshLive();
    return;
  }
  if (s.taken_over_ms) return;
  if (!(await control(s, "takeover"))) return;
  await refreshLive();
  // Steering is done on the large screen.
  if (s.watchable) openZoom(liveSession(key) || s, opener);
}

// Where on the shown picture (object-fit: contain) a pointer is, as 0–1 fractions.
function fraction(img, e) {
  const r = img.getBoundingClientRect();
  const nw = img.naturalWidth || r.width, nh = img.naturalHeight || r.height;
  const k = Math.min(r.width / nw, r.height / nh);
  const w = nw * k, hgt = nh * k;
  const x = (e.clientX - r.left - (r.width - w) / 2) / w, y = (e.clientY - r.top - (r.height - hgt) / 2) / hgt;
  return x >= 0 && x <= 1 && y >= 0 && y <= 1 ? { x, y } : null;
}

// Click = tap, drag = swipe, on the large screen while the session is taken over.
function steerable(box, key) {
  const img = box.querySelector("img.screen");
  if (!img) return;
  let down = null;
  img.addEventListener("pointerdown", (e) => {
    const s = liveSession(key);
    if (!s || !s.taken_over_here) return;
    e.preventDefault();
    down = fraction(img, e);
  });
  img.addEventListener("pointerup", async (e) => {
    const s = liveSession(key);
    const from = down;
    down = null;
    if (!s || !s.taken_over_here || !from) return;
    const to = fraction(img, e);
    if (!to) return;
    const r = img.getBoundingClientRect();
    const moved = Math.hypot((to.x - from.x) * r.width, (to.y - from.y) * r.height);
    if (moved < 12) await control(s, "tap", from);
    else await control(s, "swipe", { x: from.x, y: from.y, x2: to.x, y2: to.y });
  });
}

function steerBar(key) {
  const s = liveSession(key);
  const text = h("input", { type: "text", class: "steer-text", maxlength: "500", placeholder: "Type into the focused field", "aria-label": "Text to type on the device" });
  const sendText = async () => {
    const s2 = liveSession(key);
    if (s2 && text.value && await control(s2, "type", { text: text.value })) text.value = "";
  };
  text.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); sendText(); } });
  const key_ = (k, label) => h("button", { class: "btn sm", onclick: () => { const s2 = liveSession(key); if (s2) control(s2, "key", { key: k }); } }, label);
  const back = s && s.platform !== "web"
    ? h("button", { class: "btn sm", "aria-label": "The phone's Back button", onclick: () => { const s2 = liveSession(key); if (s2) control(s2, "back"); } }, "◁ Back (HP)")
    : null;
  const on = h("div", { class: "steer-on" },
    h("p", { class: "muted small" }, "Click the screen to tap, drag to swipe. The AI's steps wait until you press Give back to the AI or close this screen; if this page closes, the session goes back by itself after about 45 seconds."),
    h("div", { class: "steer-row" }, text, h("button", { class: "btn sm", onclick: sendText }, "Type"), key_("Enter", "Enter ↵"), key_("Backspace", "⌫ Delete a character"), back));
  const take = h("button", { class: "btn sm", "data-fk": `take-zoom:${key}`, onclick: (e) => toggleTakeover(key, e.currentTarget) });
  const bar = h("div", { class: "steer", "data-key": key }, h("div", { class: "steer-head" }, h("span", { class: "steer-state" }), take), on);
  updateSteer(bar);
  return bar;
}

function updateSteer(bar) {
  const s = liveSession(bar.dataset.key);
  if (!s) return;
  const taken = !!s.taken_over_here;
  setTakeLabel(bar.querySelector("[data-fk^='take-zoom:']"), s);
  const close = bar.closest("dialog")?.querySelector(".zoom-close");
  const closeText = taken ? "Give back & close" : "Close";
  if (close && close.dataset.label !== closeText) { close.dataset.label = closeText; close.replaceChildren(icon("close", "sm"), closeText); }
  const state = bar.querySelector(".steer-state");
  const t = taken ? "Taken over by you · the AI's step waits" : s.taken_over_ms ? "Taken over on another page" : "Automatic · take over to click and type yourself";
  if (state.textContent !== t) state.textContent = t;
  state.className = "steer-state " + (taken ? "warn" : "muted");
  bar.querySelector(".steer-on").hidden = !taken;
  bar.closest("dialog")?.classList.toggle("steering", taken);
}

// Perbesar: a modal <dialog> (focus stays inside, Escape closes), focus goes back to the button.
let zoomOpener = null;
function openZoom(s, opener) {
  closeZoom(false);
  zoomOpener = opener;
  const dlg = h("dialog", { class: "zoom", "aria-label": `Screen of session ${s.id}, enlarged` },
    h("div", { class: "lh" }, h("span", { class: "pf " + s.platform }, s.platform), h("span", { class: "mono" }, s.id),
      h("button", { class: "btn sm right zoom-close", onclick: () => closeZoom(true, true) }, icon("close", "sm"), "Close")),
    screenEl(s, true),
    s.controllable ? steerBar(`${s.pid}:${s.id}`) : null);
  if (s.controllable) steerable(dlg.querySelector(".screen-box"), `${s.pid}:${s.id}`);
  dlg.querySelectorAll(".steer").forEach(updateSteer);
  dlg.dataset.key = `${s.pid}:${s.id}`;
  dlg.addEventListener("cancel", (e) => { e.preventDefault(); closeZoom(true, true); });
  // Tab and Shift+Tab go round inside the dialog only.
  dlg.addEventListener("keydown", (e) => {
    if (e.key !== "Tab") return;
    const items = [...dlg.querySelectorAll("button, input, a[href], [tabindex]:not([tabindex='-1'])")].filter((x) => !x.disabled && !x.closest("[hidden]"));
    if (!items.length) return;
    const at = items.indexOf(document.activeElement);
    const next = e.shiftKey ? (at <= 0 ? items.length - 1 : at - 1) : (at < 0 || at === items.length - 1 ? 0 : at + 1);
    e.preventDefault();
    items[next].focus();
  });
  document.body.append(dlg);
  dlg.showModal();
  dlg.querySelector("button").focus();
}
// `release`: the user closed it; a session they took over goes back to the AI (no hanging takeover).
function closeZoom(restore, release) {
  document.querySelectorAll("dialog.zoom").forEach((d) => {
    const s = release ? liveSession(d.dataset.key) : null;
    if (s && s.taken_over_here) control(s, "release").then(refreshLive);
    stopWatching(d); if (d.open) d.close(); d.remove();
  });
  if (restore && zoomOpener && zoomOpener.isConnected) zoomOpener.focus();
  zoomOpener = null;
}

function livePill(sessions) {
  if (!sessions.length) return h("span", { class: "pill grey" }, "○ No session is running");
  const count = (st) => sessions.filter((s) => s.flow && s.flow.steps.some((x) => x.state === st)).length;
  const running = sessions.filter((s) => s.flow && !s.flow.finished).length;
  const blocked = count("blocked"), failed = count("failed");
  const parts = [`${sessions.length} ${sessions.length === 1 ? "session" : "sessions"}`, `${running} running`];
  const taken = sessions.filter((s) => s.taken_over_ms).length;
  if (taken) parts.push(`${taken} taken over`);
  if (blocked) parts.push(`${blocked} blocked`);
  if (failed) parts.push(`${failed} failed`);
  return h("span", { class: "pill" + (blocked || failed ? " bad" : "") }, "● " + parts.join(" · "));
}

// D10d (v7 #s2): every agent side by side; the queue with its reasons; the parallel limit and
// today's usage per agent.
// PF5 (v11 f4b): a test running once per platform: its summary row, one tile per platform with its
// own state (one device runs one test at a time, so a platform may wait while the others go on).
// Test words and notes are set as text.
function sideRow(r) {
  const tiles = r.tiles || [];
  const finished = tiles.length && tiles.every((t) => ["passed", "failed", "not_run"].includes(t.state));
  const at = new Date(r.started_ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const tile = (t) => {
    const n = t.state === "passed" ? t.steps : Math.min(t.step, t.steps);
    let state;
    if (t.state === "passed") state = [h("span", { class: "sbs-chip ok" }, t.step === t.steps ? `✓ Done · ${t.steps} of ${t.steps}` : "✓ Passed")];
    else if (t.state === "failed") state = [h("span", { class: "sbs-chip bad" }, "✗ Failed"), t.note ? " " + t.note : null];
    else if (t.state === "not_run") state = [h("span", { class: "sbs-chip grey" }, "Not run"), " ", t.note || ""];
    else if (t.state === "waiting") state = [h("span", { class: "sbs-chip warn" }, "waiting"), " ", t.note || ""];
    else state = [h("span", { class: "sbs-chip ok" }, t.step ? `Step ${t.step} of ${t.steps}` : "Starting"), t.label ? " " + t.label : null];
    return h("li", { class: "sbs-tile " + t.state },
      h("b", {}, t.name),
      h("progress", { max: String(Math.max(1, t.steps)), value: String(n), "aria-label": `${t.name}: ${n} of ${t.steps} steps` }),
      h("span", { class: "s" }, ...state));
  };
  return h("section", { class: "grp sbs-live", "aria-label": `${r.title} on ${tiles.length} platforms` },
    h("div", { class: "gh" }, h("b", {}, `${r.title} · on ${tiles.length} platforms`),
      h("span", { class: "sbs-chip " + (finished ? "grey" : "ok") }, finished ? "done" : "running"), h("span", { class: "right s" }, `started ${at}`)),
    h("ul", { class: "sbs-tiles" }, tiles.map(tile)),
    h("p", { class: "s sbs-foot" }, "The side by side report opens when all platforms are done: find it in ", h("a", { href: "#/history" }, "History"), "."),
    // S8: run it again on every platform of the app.
    finished ? h("div", { class: "btns tight" }, ...runAllButtons(r.id)) : null);
}

function liveExtras(data) {
  const box = document.getElementById("live-extra") || h("div", { id: "live-extra", class: "live-extra" });
  const q = data.queue || [];
  const u = data.usage || [];
  const side = data.side_by_side || [];
  const sig = JSON.stringify([q, u, side]);
  if (box.dataset.sig === sig) return box;
  box.dataset.sig = sig;
  box.replaceChildren(...[
    ...side.map(sideRow),
    q.length ? h("section", { class: "grp" }, h("div", { class: "gh" }, `queue · ${q.length} waiting`),
      h("ul", { class: "ag-rows q-rows" }, q.map((e) => h("li", {}, h("b", {}, e.agent || "—", h("span", { class: "s" }, ` · ${e.platform}${e.device && e.device !== "default" ? " · " + e.device : ""}${e.session !== "default" ? " · " + e.session : ""}`)),
        h("span", { class: "s" }, `waiting ${Math.max(1, Math.round((Date.now() - e.since_ms) / 60000))} min`), h("span", {}, e.reason))))) : null,
    u.length ? h("section", { class: "grp" }, h("div", { class: "gh" }, "used today, per agent"),
      h("ul", { class: "ag-rows" }, u.map((x) => h("li", {}, h("b", {}, x.agent), h("span", {}, `${x.turns} ${x.turns === 1 ? "answer or run" : "answers and runs"}`), h("span", { class: "s" }, `${x.tokens.toLocaleString("en")} tokens`))))) : null].filter(Boolean));
  return box;
}

// "Tests at once": every value the setting allows (1 to `limit_max`, from the server), so a limit
// above 10 shows as itself instead of falling back to 1 (L062-12).
function limitPicker(data) {
  const max = Math.max(Number(data.limit_max) || 20, Number(data.limit) || 1);
  const dflt = Number(data.limit_default) || 3;
  const sel = h("select", { class: "sel", "aria-label": "Tests at once" }, Array.from({ length: max }, (_, i) => h("option", { value: String(i + 1), selected: data.limit === i + 1 }, `${i + 1}${i + 1 === dflt ? " (default)" : ""}`)));
  sel.addEventListener("change", async () => {
    try { await api("/api/live/limit", { limit: Number(sel.value) }); toast(`Up to ${sel.value} tests at once, from the next test.`, true); } catch (e) { toast(e.message); }
  });
  return h("label", { class: "right live-limit" }, h("span", { class: "s" }, "Tests at once"), sel);
}

function renderLive(data) {
  lastLive = data;
  const sessions = data.sessions || [];
  const q = (data.queue || []).length;
  // No null children: replaceChildren writes them as the text "null" (FastTest, final report).
  $top().replaceChildren(...[icon("live"), h("h1", { tabindex: "-1" }, "Live"), livePill(sessions),
    q ? h("span", { class: "pill warn" }, `${q} queued`) : null].filter(Boolean));
  if (!sessions.length) {
    stopWatching();
    liveCards.clear();
    stepsSig = "";
    closeZoom(false);
    $body().replaceChildren(liveExtras(data), liveEmpty());
    return;
  }
  const keys = new Set(sessions.map((s) => `${s.pid}:${s.id}`));
  for (const [k, c] of [...liveCards.entries()]) {
    if (!keys.has(k)) { stopWatching(c.el); c.el.remove(); liveCards.delete(k); }
  }
  if (!liveSel || !keys.has(liveSel)) liveSel = [...keys][0];
  // Built once and updated in place: moving an <img> would restart its stream.
  let root = document.getElementById("live-root");
  if (!root) {
    for (const c of liveCards.values()) stopWatching(c.el);
    liveCards.clear();
    stepsSig = "";
    root = h("div", { id: "live-root" }, h("div", { class: "lgrid", id: "live-grid" }), h("div", { id: "live-steps" }));
    $body().replaceChildren(root);
  }
  if (!root.contains(document.getElementById("live-extra"))) root.prepend(liveExtras(data)); else liveExtras(data);
  const grid = document.getElementById("live-grid");
  for (const s of sessions) {
    const k = `${s.pid}:${s.id}`;
    let c = liveCards.get(k);
    if (!c) { c = card(s); liveCards.set(k, c); grid.append(c.el); }
    fillCard(c, s, k === liveSel);
  }
  // The step list is rebuilt only when it changed, and focus inside it is put back.
  const sel = sessions.find((s) => `${s.pid}:${s.id}` === liveSel);
  const sig = JSON.stringify([liveSel, sel && sel.flow]);
  if (sig !== stepsSig) {
    stepsSig = sig;
    const host = document.getElementById("live-steps");
    const fk = host.contains(document.activeElement) ? document.activeElement.dataset.fk : null;
    host.replaceChildren(...(sel ? [stepsPanel(sel)] : []));
    if (fk) host.querySelector(`[data-fk="${fk}"]`)?.focus();
  }
  document.querySelectorAll("dialog.zoom .steer").forEach(updateSteer);
  takeBanner(sessions);
  if (document.querySelector("dialog.zoom") && zoomOpener && !zoomOpener.isConnected) closeZoom(false);
}

// A takeover from this page is visible on every page, with the way back.
function takeBanner(sessions) {
  const mine = (sessions || []).filter((s) => s.taken_over_here);
  let el = document.getElementById("take-banner");
  if (!mine.length) { if (el) el.remove(); return; }
  if (!el) {
    el = h("div", { id: "take-banner", class: "take-banner", role: "status" });
    document.getElementById("main").prepend(el);
  }
  const sig = mine.map((s) => `${s.pid}:${s.id}`).join(",");
  if (el.dataset.sig === sig) return;
  el.dataset.sig = sig;
  const names = mine.map((s) => `${s.id} (${s.platform})`).join(", ");
  el.replaceChildren(icon("cursor", "sm"),
    h("span", {}, h("b", {}, "You are taking over "), names, ". The AI's steps in that session wait."),
    h("a", { class: "btn sm", href: "#/live" }, "See it in Live"),
    h("button", { class: "btn sm p", onclick: async () => {
      for (const s of mine) await control(s, "release");
      await refreshLive();
      await globalLive();
      // The banner (and this button) is gone: keyboard focus goes to the page title.
      document.querySelector(".top h1")?.focus();
    } }, icon("play", "sm"), "Give back to the AI"));
}

// Off the Live page: the Live state every few seconds, for the banner (and the takeover's keep-alive).
async function globalLive() {
  if ((location.hash || HOME).startsWith("#/live")) return;
  try { lastLive = await api("/api/live"); takeBanner(lastLive.sessions); } catch (_) { /* next time */ }
}
setInterval(globalLive, 4000);

async function pollLive() {
  if (!(location.hash || HOME).startsWith("#/live")) return;
  try {
    renderLive(await api("/api/live"));
  } catch (e) {
    $body().replaceChildren(h("div", { class: "err", role: "alert" }, "Cannot read the sessions: ", rich(e.message)));
  }
  liveTimer = setTimeout(pollLive, 1000);
}

// ---- Chat (C4) ----
let chatData = null, chatTimer = null, chatSig = "";
const chatEdits = {}; // proposal id -> what the user is changing with Edit

// D10a: whose conversation Chat shows, and where it draws (agents.js puts it in an agent's Chat tab;
// null = the Chat page itself).
let chatAgent = "default", chatHost = null;
const chatQ = () => `?agent=${encodeURIComponent(chatAgent)}`;
const chatShown = () => chatHost !== null && document.body.contains(chatHost);

async function loadChat() {
  const agent = chatAgent;
  let data;
  try { data = await api("/api/chat" + chatQ()); } catch (e) { data = { error: e.message, messages: [], busy: false }; }
  if (agent !== chatAgent) return; // another agent was opened meanwhile
  chatData = data;
  if (chatShown()) renderChat();
  clearTimeout(chatTimer);
  if (chatData.busy && chatShown()) chatTimer = setTimeout(loadChat, 800);
}

// PF6: text to put in the Chat box (not sent) when Chat opens next.
let chatPrefill = null;

const CHAT_SUGGESTIONS = ["Check the login", "Map the app", "Can we release?"];

// C1: "or just say it" under a page: up to three fixed sentences, written by fastpilot (never taken
// from a page), each sent to Chat with one click; Chat answers with a card that names its action.
function sayIt(sentences) {
  return h("section", { class: "grp limit-say", "aria-label": "Or just say it in Chat" },
    h("div", { class: "gh" }, "or just say it in chat"),
    h("div", { class: "prop-body" },
      sentences.slice(0, 3).map((t) => h("button", { type: "button", class: "say-row", onclick: () => openChat(t) }, h("span", {}, t), h("span", { class: "arrow", "aria-hidden": "true" }, "→")))));
}

// U1: one Chat page, the menu's (the default agent's conversation). Buttons elsewhere open it, and
// may send a sentence there.
async function openChat(send) {
  chatAgent = "default";
  // Already there (the first-run wizard covers it): no hash change, so draw it now.
  if (location.hash === "#/agents/default/chat") render(); else location.hash = "#/agents/default/chat";
  if (send) await chatSend(send);
}

async function chatSend(text) {
  const t = (text || "").trim();
  if (!t) return;
  try {
    chatData = await api("/api/chat", { agent: chatAgent, message: t });
    const box = document.getElementById("chat-input");
    if (box) box.value = "";
    chatDraft.text = "";
    clearToast();
  } catch (e) { toast(e.message); }
  loadChat();
}

// Why a card's last click was refused, shown in that card until it works (FastTest M3: a refused
// "Save test only" on a duplicate gave no feedback where the person clicked).
const chatRefused = {};
async function chatDecide(id, apply, mode, edit, confirm) {
  try {
    const body = { id, apply, agent: chatAgent };
    if (mode) body.mode = mode;
    if (edit) body.edit = edit;
    if (confirm !== undefined) body.confirm = confirm;
    const r = await api("/api/chat/decide", body);
    delete chatRefused[id];
    delete chatEdits[id];
    chatData = r.chat;
    toast(r.message, true);
  } catch (e) { chatRefused[id] = e.message; toast(e.message); await loadChat(); }
  chatSig = "";
  renderChat();
  if (chatRefused[id]) document.querySelector(`.prop[data-pid="${id}"] .prop-refused`)?.focus();
  // The clicked button is gone: keyboard focus goes to the card's new state.
  document.querySelector(`.prop[data-pid="${id}"] .prop-state`)?.focus();
}

// `kode` in a message becomes a code span; everything else stays plain text.
function rich(text) {
  return String(text || "").split(/`([^`\n]+)`/).map((part, i) => (i % 2 ? code(part) : part));
}

// `**bold**` inside one line of the bot's text, around `code`.
function inline(text) {
  return String(text || "").split(/`([^`\n]+)`/).flatMap((part, i) =>
    i % 2 ? [code(part)] : part.split(/\*\*([^*\n]+)\*\*/).map((x, j) => (j % 2 ? h("b", {}, x) : x)));
}

// The bot's answer (QA B6): fenced blocks as monospace blocks (also one Claude did not close),
// "# headings" as bold lines, **bold**, `code`; everything else as plain text. Built as DOM text
// nodes, never as HTML.
function markdown(text) {
  const out = [];
  const parts = String(text || "").split(/^```[^\n]*$/m);
  parts.forEach((part, i) => {
    // The line break right after a fence and right before the next one belong to the fence.
    let body = part;
    if (i > 0 && body.startsWith("\n")) body = body.slice(1);
    if (i < parts.length - 1 && body.endsWith("\n")) body = body.slice(0, -1);
    if (i % 2) {
      out.push(h("pre", { class: "md-pre" }, body));
      return;
    }
    if (!body) return;
    body.split("\n").forEach((line, k) => {
      if (k) out.push("\n");
      const head = line.match(/^#{1,6}\s+(.*)$/);
      out.push(...(head ? [h("b", { class: "md-h" }, inline(head[1]))] : inline(line)));
    });
  });
  return out;
}

// S8: "Run on all platforms" on a saved test: every platform its app has, side by side. A platform
// the app does not have is an "Add Android app" button. What it would run is read once per test.
const runAllInfo = {};
function runAllButtons(test) {
  if (!test) return [];
  const info = runAllInfo[test];
  if (info === undefined) {
    runAllInfo[test] = null;
    api("/api/run-all?test=" + encodeURIComponent(test)).then((v) => {
      runAllInfo[test] = v;
      // Draw again where the buttons are: the Chat card now, Live on its next refresh.
      const live = document.getElementById("live-extra");
      if (live) live.dataset.sig = "";
      if ((location.hash || "").includes("/chat")) { chatSig = ""; renderChat(); }
    }).catch(() => { runAllInfo[test] = { platforms: [], missing: [] }; });
    return [];
  }
  if (!info) return [];
  const go = info.platforms.length >= 2
    ? [h("button", { class: "btn", title: "On " + info.platforms.join(", "), onclick: () => runAll(test) }, icon("play", "sm"), "Run on all platforms")]
    : [];
  return [...go, ...info.missing.map((m) => h("a", { class: "btn quiet", href: m.href, "data-kind": m.kind }, icon("plus", "sm"), m.button))];
}

async function runAll(test) {
  try { toast((await api("/api/run-all", { test })).message, true); } catch (e) { toast(e.message); }
}

async function chatRun(id) {
  try {
    const r = await api("/api/chat/run", { id, agent: chatAgent });
    chatData = r.chat;
    toast(r.message, true);
  } catch (e) { toast(e.message); }
  chatSig = "";
  renderChat();
  loadChat();
}

// S5: one set of schedule presets everywhere (Design Critic M1).
const SCHED_PRESETS = [["two-hours", "Every 2 hours (8–20)"], ["night", "Every night"], ["release", "Before each release"]];
// A test the bot wrote (D1, v7 #s1): one card, saved only when the user clicks.
function testCard(p) {
  const ed = chatEdits[p.id];
  const head = h("h3", { class: "gh" }, icon("tests"), h("span", {}, p.schedules ? "Save test & schedule" : "Save test", " · "), h("b", { class: "t-title" }, p.title));
  let body;
  if (ed) {
    const title = h("input", { type: "text", maxlength: "80", value: ed.title, "aria-label": "Test title" });
    const steps = h("textarea", { rows: String(Math.max(3, ed.steps.length + 1)), "aria-label": "Steps, one per line" }, ed.steps.join("\n"));
    const target = h("input", { type: "text", maxlength: "500", value: ed.target, "aria-label": "Address or app" });
    // PF5: a test on several platforms has an address or app per platform.
    const names = { web: "Address on web", android: "App on Android", ios: "App on iPhone" };
    const per = ed.targets ? Object.keys(names).filter((k) => k in ed.targets).map((k) => [k, h("input", { type: "text", maxlength: "500", value: ed.targets[k], "aria-label": names[k] })]) : [];
    const file = h("input", { type: "text", maxlength: "48", value: ed.id, class: "mono", "aria-label": "File name" });
    const read = () => Object.assign({ title: title.value, target: target.value, id: file.value, steps: steps.value.split("\n").map((x) => x.trim()).filter(Boolean) },
      per.length ? { targets: Object.fromEntries(per.map(([k, el]) => [k, el.value])) } : {});
    // Typed text survives a refresh of the conversation while the bot answers.
    [title, steps, target, file, ...per.map((x) => x[1])].forEach((el) => el.addEventListener("input", () => { chatEdits[p.id] = read(); }));
    body = h("div", { class: "form t-edit" },
      h("label", { class: "fld" }, h("span", { class: "l" }, "Title"), title),
      h("label", { class: "fld" }, h("span", { class: "l" }, "Steps"), steps, h("span", { class: "s" }, "One step per line, in plain words. Secrets only as ${NAME}.")),
      per.length ? per.map(([k, el]) => h("label", { class: "fld" }, h("span", { class: "l" }, names[k]), el)) : h("label", { class: "fld" }, h("span", { class: "l" }, "Address or app"), target),
      h("label", { class: "fld" }, h("span", { class: "l" }, "File name"), file),
      h("div", { class: "btns" },
        p.schedules ? h("button", { class: "btn p", onclick: () => chatDecide(p.id, true, "schedule", read()) }, "Save & schedule") : null,
        h("button", { class: p.schedules ? "btn" : "btn p", onclick: () => chatDecide(p.id, true, "test", read()) }, "Save test only"),
        h("button", { class: "btn", onclick: () => { delete chatEdits[p.id]; chatSig = ""; renderChat(); } }, "Cancel edit")));
  } else {
    body = [
      h("ul", { class: "t-steps", "aria-label": "Steps" }, p.steps.map((x) => h("li", {}, x))),
      p.rows.length ? h("dl", { class: "prop-rows" }, p.rows.map(([k, v]) => [h("dt", {}, k), h("dd", {}, v)])) : null,
    ];
  }
  let foot = null;
  const refusedNow = p.status === "pending" && chatRefused[p.id] ? h("div", { class: "prop-state bad prop-refused", role: "alert", tabindex: "-1" }, "Not saved: ", rich(chatRefused[p.id])) : null;
  if (p.status === "pending" && !ed) {
    foot = h("div", { class: "btns" },
      p.schedules ? h("button", { class: "btn p", onclick: () => chatDecide(p.id, true, "schedule") }, "Save & schedule") : null,
      h("button", { class: p.schedules ? "btn" : "btn p", onclick: () => chatDecide(p.id, true, "test") }, "Save test only"),
      h("button", { class: "btn", onclick: () => { chatEdits[p.id] = JSON.parse(JSON.stringify(p.draft)); chatSig = ""; renderChat(); document.querySelector(`.prop[data-pid="${p.id}"] input`)?.focus(); } }, "Edit"),
      h("button", { class: "btn quiet", onclick: () => chatDecide(p.id, false) }, "Dismiss"),
      // S5: save the test and run it by itself, in one click (a Mac notification when it fails).
      h("div", { class: "presets-inline", role: "group", "aria-label": "Save and run it by itself" }, h("span", { class: "s" }, "Save and run it by itself:"),
        SCHED_PRESETS.map(([k, t]) => h("button", { class: "btn sm", onclick: () => chatDecide(p.id, true, "preset:" + k) }, t))));
  } else if (p.status === "applied") {
    foot = [h("div", { class: "prop-state ok", tabindex: "-1" }, "✓ ", rich(p.note || "Saved")),
      h("div", { class: "btns" }, h("button", { class: "btn", onclick: () => chatRun(p.id) }, icon("play", "sm"), "Run now"),
        ...runAllButtons(p.saved && p.saved.test),
        p.saved && p.saved.schedule ? h("a", { class: "btn", href: "#/schedules" }, "Open Schedules") : null, h("a", { class: "btn", href: "#/live" }, "Open Live"))];
  } else if (p.status === "cancelled") {
    foot = h("div", { class: "prop-state muted", tabindex: "-1" }, "Dismissed");
  }
  const note = p.status === "pending" && p.note ? h("p", { class: "s warnline" }, rich(p.note)) : null;
  const detail = p.detail ? h("details", { class: "t-detail" }, h("summary", {}, "Details"), h("p", { class: "s" }, p.detail)) : null;
  return h("section", { class: "grp prop test " + p.status, "aria-label": (p.schedules ? "Save test & schedule: " : "Save test: ") + p.title, "data-pid": String(p.id) },
    head, h("div", { class: "prop-body" }, body, note, detail, refusedNow, foot));
}

async function chatUndo(id) {
  try {
    const r = await api("/api/chat/undo", { id, agent: chatAgent });
    chatData = r.chat;
    toast(r.message, true);
  } catch (e) { toast(e.message); }
  chatSig = "";
  renderChat();
  document.querySelector(`.prop[data-pid="${id}"] .prop-state`)?.focus();
}

// D9 step 2 (v9 G2): allowing a looser limit that needs a typed word. Focus starts on Cancel (the
// safe choice), Escape cancels, and Allow is enabled only after the word is typed and ~3 seconds.
function confirmLoosen(p, back) {
  const c = p.setting;
  const input = h("input", { type: "text", autocomplete: "off", spellcheck: "false", "aria-label": `Type ${c.typed} to confirm` });
  const allow = h("button", { class: "btn danger-fill", disabled: true }, `${c.title.replace(/^Allow /, "Allow ")} from the next run`);
  let waited = false;
  const sync = () => { allow.disabled = !(waited && input.value.trim().toLowerCase() === c.typed.toLowerCase()); };
  setTimeout(() => { waited = true; sync(); }, 3000);
  input.addEventListener("input", sync);
  const cancel = h("button", { class: "btn" }, "Cancel");
  const dlg = h("dialog", { class: "loosen-dlg", "aria-labelledby": "loosen-t" },
    h("p", { class: "step" }, "Step 2 · confirm"),
    h("h2", { id: "loosen-t" }, c.title + "?"),
    h("p", {}, c.consequence, " You can change it back at any time in Settings."),
    h("label", { class: "fld" }, h("span", { class: "l" }, "Type ", h("b", {}, c.typed), " to confirm."), input),
    h("div", { class: "btns" }, cancel, allow),
    h("p", { class: "s muted" }, "Focus starts on Cancel, the safe choice. Esc also cancels."));
  const close = () => { dlg.close(); dlg.remove(); back?.focus(); };
  cancel.addEventListener("click", close);
  dlg.addEventListener("cancel", (e) => { e.preventDefault(); close(); });
  input.addEventListener("keydown", (e) => { if (e.key === "Enter" && !allow.disabled) allow.click(); });
  allow.addEventListener("click", async () => { const word = input.value; close(); await chatDecide(p.id, true, null, null, word); });
  document.body.append(dlg);
  dlg.showModal();
  cancel.focus();
}

const LEVEL = { applied: ["Applied now", "lvl-ok"], ok: ["Needs your OK", "lvl-blue"], loosen: ["Loosens a limit", "lvl-red"] };
const STATE = { declined: "Kept as it was", expired: "Expired", undone: "Undone", failed: "Not saved", refused: "Not changed" };

// D9 (v9 G1–G3): a settings card. Every word on it comes from fastpilot, not from the bot (M3).
function settingCard(p) {
  const c = p.setting;
  const [badge, cls] = p.status === "applied" && c.level === "loosen" ? ["Allowed · next run", "lvl-ok"] : p.status !== "pending" && p.status !== "applied" ? [STATE[p.status] || p.status, "lvl-grey"] : LEVEL[c.level];
  const red = c.level === "loosen";
  const head = h("h3", { class: "gh" }, icon(red ? "alert" : c.level === "ok" ? "settings" : "shield"), h("span", { class: "s-title" }, c.title), h("span", { class: "lvl " + cls }, badge));
  const values = h("div", { class: "ba" },
    h("div", { class: "box" }, h("small", {}, "Now"), h("span", { class: "v" }, c.now)),
    h("span", { class: "arrow", "aria-hidden": "true" }, "→"),
    h("div", { class: "box after " + (red ? "red" : c.level === "ok" ? "blue" : "green") }, h("small", {}, "After"), h("span", { class: "v" }, c.after)));
  const parts = [values, h("p", { class: red ? "conseq red" : "conseq" }, c.consequence)];
  (c.notes || []).forEach((n) => parts.push(h("p", { class: "s" }, n)));
  if (red && p.quote) parts.push(h("p", { class: "quote" }, p.quote));
  if (p.note) parts.push(h("p", { class: "s" + (p.status === "pending" && red ? " warnline" : "") }, rich(p.note)));
  let foot = null;
  if (p.status === "pending" && red) {
    const review = h("button", { class: "btn danger" }, c.typed ? "Review and allow…" : "Allow");
    if (!c.typed) { review.disabled = true; setTimeout(() => { review.disabled = false; }, 3000); }
    review.addEventListener("click", () => (c.typed ? confirmLoosen(p, review) : chatDecide(p.id, true)));
    foot = h("div", { class: "btns" }, h("button", { class: "btn p", onclick: () => chatDecide(p.id, false) }, "Keep it"), review);
  } else if (p.status === "pending") {
    foot = h("div", { class: "btns" }, h("button", { class: "btn p", onclick: () => chatDecide(p.id, true) }, "Save changes"), h("button", { class: "btn", onclick: () => chatDecide(p.id, false) }, "Keep it"));
  } else if (p.status === "applied" && p.undo) {
    foot = h("div", { class: "btns" }, h("div", { class: "prop-state ok", tabindex: "-1" }, red ? "Allowed" : "Done"),
      h("button", { class: "btn sm", onclick: () => chatUndo(p.id) }, red ? "Block again" : "Undo"));
  } else {
    foot = h("div", { class: "prop-state muted", tabindex: "-1" }, STATE[p.status] || "");
  }
  return h("section", { class: "grp prop setting " + (red ? "loosen " : "") + p.status, "aria-label": c.title, "data-pid": String(p.id) }, head, h("div", { class: "prop-body" }, parts, foot));
}

// M1: a looser limit the bot proposed that the user did not ask for. Nothing was changed.
// A loosening the user's own words said no to ("don't allow Pay") is kept apart: it was their words.
function pageAttempts(list) {
  const mine = list.filter((a) => a.kind === "said_no");
  const page = list.filter((a) => a.kind !== "said_no");
  const out = [];
  if (mine.length) {
    out.push(h("div", { class: "page-try", role: "status" }, icon("info", "sm"),
      h("div", {}, h("b", {}, "Limit kept as it is. "), "Your message names it but does not ask for it (it says no, or asks why): ", mine.map((a) => a.what).join(", "), ". Nothing was changed.")));
  }
  // One box per kind of attempt: a rule (note), a Jira write, a limit.
  const groups = [
    [page.filter((a) => a.kind === "note"), [h("b", {}, "A page tried to give the agent a rule. "), "Nothing was saved. Only your own messages and the Memory page can add notes."]],
    [page.filter((a) => a.kind === "jira" && a.from !== "none" && a.from !== "claim"), [h("b", {}, "A ticket or page asked me to write to Jira. "), "Nothing was done. Only your own messages can do that."]],
    [page.filter((a) => a.kind !== "note" && a.kind !== "jira"), [h("b", {}, "A page tried to change a limit. "), "Nothing was changed. Only your own messages can ask for this."]],
  ];
  // QA bug 1: the bot drafted a Jira write the user's message did not ask for in so many words.
  // No card was made: say so plainly, with the words that get the card. The draft is plain text.
  page.filter((a) => a.kind === "jira" && a.from === "none").forEach((a) => {
    out.push(h("div", { class: "page-try", role: "status" }, icon("info", "sm"),
      h("div", {}, h("b", {}, "No Jira card was made. "), "Your message did not ask for one in your own words, so nothing was sent. ",
        a.say ? ["Say “", a.say, "” to get the card", a.offer ? ", or reply “yes” to get a card for this draft." : "."] : null,
        h("details", {}, h("summary", {}, "See the bot's draft"), h("pre", {}, a.said)))));
  });
  // L061-1/2: the bot wrote that it filed or changed a Jira card, but fastpilot made none.
  page.filter((a) => a.kind === "jira" && a.from === "claim").forEach((a) => {
    out.push(h("div", { class: "page-try", role: "status" }, icon("info", "sm"),
      h("div", {}, h("b", {}, "No Jira card was made in this answer. "), "The bot wrote that it filed or changed one, which was not true, so those words were taken out. Nothing was sent to Jira.",
        h("details", {}, h("summary", {}, "See the words taken out"), h("pre", {}, a.said)))));
  });
  groups.forEach(([list, words]) => {
    if (!list.length) return;
    out.push(h("div", { class: "page-try", role: "status" }, icon("alert", "sm"),
      h("div", {}, words,
        h("details", {}, h("summary", {}, "See what it said"),
          list.map((a) => h("div", { class: "said" }, h("p", { class: "s" }, a.what, " (not asked by you, untrusted text):"), h("pre", {}, a.said)))))));
  });
  return out;
}

// D10a: "New agent · …" (v7 #s4, right): created only on Create; Edit opens the form filled in.
function agentCard(p) {
  const rows = h("dl", { class: "prop-rows" }, p.rows.map(([k, v]) => [h("dt", {}, k), h("dd", {}, v)]));
  let foot;
  if (p.status === "pending") {
    foot = h("div", { class: "btns" },
      h("button", { class: "btn p", onclick: () => chatDecide(p.id, true) }, "Create agent"),
      h("button", { class: "btn", onclick: () => window.editAgentDraft && window.editAgentDraft(p.draft || {}) }, "Edit"),
      h("button", { class: "btn quiet", onclick: () => chatDecide(p.id, false) }, "Dismiss"));
  } else if (p.status === "applied") {
    foot = h("div", { class: "btns" }, h("div", { class: "prop-state ok", tabindex: "-1" }, "✓ ", p.note || "Created"),
      p.saved && p.saved.agent ? h("a", { class: "btn", href: `#/agents/${encodeURIComponent(p.saved.agent)}/chat` }, "Open the agent") : null);
  } else {
    foot = h("div", { class: "prop-state muted", tabindex: "-1" }, p.status === "cancelled" ? "Dismissed" : (p.note || ""));
  }
  return h("section", { class: "grp prop agent " + p.status, "aria-label": p.title, "data-pid": String(p.id) },
    h("h3", { class: "gh" }, icon("plus"), h("b", {}, p.title)), h("div", { class: "prop-body" }, rows, p.status === "pending" && p.note ? h("p", { class: "s" }, p.note) : null, foot));
}

// v0.6 (design 04): "Create Jira issue" and "Comment on KAN-12". Every word on the card comes from
// fastpilot; the text shown is exactly what is sent, and nothing is sent before the click.
function jiraText(text) {
  const t = String(text || "");
  if (t.length <= 400) return h("pre", { class: "jira-pre" }, t);
  return h("div", {}, h("pre", { class: "jira-pre" }, t.slice(0, 400) + "…"),
    h("details", {}, h("summary", {}, "Show all"), h("pre", { class: "jira-pre" }, t)));
}

function jiraLink(j, url, label) {
  // Only a link to the connected site's own issue page.
  const ok = typeof url === "string" && j.site && url.startsWith(`https://${j.site}/browse/`);
  return ok ? h("a", { class: "lk", href: url, target: "_blank", rel: "noopener noreferrer" }, label || "Open in Jira") : null;
}

function jiraCard(p) {
  const j = p.jira || {};
  const issue = p.kind === "jira_issue";
  const pending = p.status === "pending";
  const ed = pending ? chatEdits[p.id] : null;
  const done = p.status === "applied";
  const commented = done && !!(j.result && j.result.comment);
  const title = done ? (commented ? "Comment added" : "Jira issue created") : p.title;
  const badge = pending ? h("span", { class: "tag" }, "Needs your OK") : done ? h("span", { class: "tag ok-tag" }, "Done")
    : p.status === "cancelled" ? h("span", { class: "tag" }, "Cancelled") : p.status === "expired" ? h("span", { class: "tag" }, "Expired") : null;
  const head = h("h3", { class: "gh" }, icon("plug"), h("span", { class: "s-title" }, title), badge);
  const whole = issue ? `${j.summary || ""}\n\n${j.description || ""}`.trim() : (j.text || "");
  const copy = h("button", { class: "btn sm", onclick: () => copyText(j.text || whole, "Text") }, "Copy the text");
  const parts = [];
  if (p.quote && (pending || p.status === "sending")) parts.push(h("p", { class: "quote" }, p.quote));
  if (j.not_connected) {
    parts.push(jiraText(j.text));
    parts.push(h("div", { class: "prop-state bad", role: "status", tabindex: "-1" }, rich(p.note || "Jira is not connected."), " ",
      h("a", { class: "lk", href: "#/integrations/jira" }, "Connect Jira in Integrations"), " ", copy));
    return h("section", { class: "grp prop jira " + p.status, "aria-label": p.title, "data-pid": String(p.id) }, head, h("div", { class: "prop-body" }, parts));
  }
  if (p.status === "refused") {
    parts.push(h("div", { class: "prop-state bad", tabindex: "-1" }, rich(p.note || "This cannot be sent.")));
    return h("section", { class: "grp prop jira refused", "aria-label": p.title, "data-pid": String(p.id) }, head, h("div", { class: "prop-body" }, parts));
  }
  if (ed) {
    const read = [];
    const fields = issue
      ? [["summary", "Summary", h("input", { type: "text", maxlength: "250", value: ed.summary || "", "aria-label": "Summary" })],
        ["description", "Description", h("textarea", { rows: "8", "aria-label": "Description" }, ed.description || "")]]
      : [["text", "Comment", h("textarea", { rows: "6", "aria-label": "Comment" }, ed.text || "")]];
    const value = () => Object.fromEntries(fields.map(([k, , el]) => [k, el.value]));
    fields.forEach(([, , el]) => el.addEventListener("input", () => { chatEdits[p.id] = value(); }));
    fields.forEach(([, label, el]) => read.push(h("label", { class: "fld" }, h("span", { class: "l" }, label), el)));
    parts.push(h("div", { class: "form t-edit" }, read,
      h("p", { class: "s" }, "fastpilot cleans the text again before sending: secrets masked, no emails, no links to other sites."),
      h("div", { class: "btns" },
        h("button", { class: "btn p", onclick: () => chatDecide(p.id, true, null, value()) }, issue ? "Create issue" : "Add comment"),
        h("button", { class: "btn", onclick: () => { delete chatEdits[p.id]; chatSig = ""; renderChat(); } }, "Cancel edit"))));
  } else if (issue) {
    parts.push(h("dl", { class: "prop-rows" },
      h("dt", {}, "Site"), h("dd", {}, j.site || ""),
      h("dt", {}, "Project"), h("dd", {}, `${j.project_name || j.project} (${j.project})`),
      h("dt", {}, "Summary"), h("dd", {}, j.summary || ""),
      h("dt", {}, "Description"), h("dd", {}, j.description ? jiraText(j.description) : h("span", { class: "muted" }, "(none)"))));
  } else {
    parts.push(h("p", { class: "jira-where" }, `${j.site || ""} · ${j.key || ""}`, j.ticket ? ` “${j.ticket}”` : ""));
    parts.push(h("dl", { class: "prop-rows" }, h("dt", {}, "Comment"), h("dd", {}, jiraText(j.text))));
  }
  if (pending && chatRefused[p.id]) {
    parts.push(h("div", { class: "prop-state bad prop-refused", role: "alert", tabindex: "-1" }, "Not sent: ", rich(chatRefused[p.id]), " ", copy));
  }
  let foot = null;
  if (pending && j.dup && !ed) {
    parts.push(h("div", { class: "jira-dup", role: "status" }, icon("alert", "sm"),
      h("div", {}, h("b", {}, `An open issue looks the same: ${j.dup.key}`), j.dup.summary ? ` “${j.dup.summary}”. ` : ". ", "Nothing was created yet. ", jiraLink(j, j.dup.url))));
    foot = h("div", { class: "btns" },
      h("button", { class: "btn p", onclick: () => chatDecide(p.id, true, "comment") }, `Comment on ${j.dup.key} instead`),
      h("button", { class: "btn", onclick: () => chatDecide(p.id, true, "anyway") }, "Create anyway"),
      h("button", { class: "btn quiet", onclick: () => chatDecide(p.id, false) }, "Cancel"));
  } else if (pending && !ed) {
    foot = h("div", { class: "btns" },
      h("button", { class: "btn p", onclick: () => chatDecide(p.id, true) }, issue ? "Create issue" : "Add comment"),
      h("button", { class: "btn", onclick: () => { chatEdits[p.id] = issue ? { summary: j.summary || "", description: j.description || "" } : { text: j.text || "" }; chatSig = ""; renderChat(); } }, "Edit first"),
      h("button", { class: "btn quiet", onclick: () => chatDecide(p.id, false) }, "Cancel"));
  } else if (p.status === "unknown") {
    // Security Low 3: sent, but no answer from Jira: check before sending again.
    const check = jiraLink(j, j.check_url, "Check in Jira");
    foot = h("div", {},
      h("div", { class: "jira-dup", role: "alert", tabindex: "-1" }, icon("alert", "sm"), h("div", {}, h("b", {}, "No answer from Jira. "), rich(p.note || ""), " ", check)),
      h("div", { class: "btns" },
        h("button", { class: "btn", onclick: () => chatDecide(p.id, true, "again") }, "Send again"),
        h("button", { class: "btn quiet", onclick: () => chatDecide(p.id, false) }, "Cancel")));
  } else if (p.status === "sending") {
    foot = h("div", { class: "prop-state muted", tabindex: "-1" }, "Sending to Jira…");
  } else if (done) {
    foot = h("div", { class: "prop-state ok", tabindex: "-1" }, "✓ ", rich(p.note || "Done"), " ", jiraLink(j, j.result && j.result.url));
  } else if (!pending) {
    foot = h("div", { class: "prop-state muted", tabindex: "-1" }, rich(p.note || "Nothing was sent to Jira."));
  }
  const note = pending && p.note && !ed ? h("p", { class: "s" }, p.note) : null;
  return h("section", { class: "grp prop jira " + p.status, "aria-label": title, "data-pid": String(p.id) }, head, h("div", { class: "prop-body" }, parts, note, foot));
}

// Tickets fastpilot read for a message (design 04 states 10 and 12): data, never instructions.
function jiraReads(list) {
  return list.map((r) => r.error
    ? h("div", { class: "page-try", role: "status" }, icon("alert", "sm"), h("div", {}, rich(r.error)))
    : h("div", { class: "jira-read", role: "status" }, icon("info", "sm"),
      h("div", {}, h("b", {}, `Read ${r.key}: `, r.summary || ""), h("p", { class: "s" }, "Ticket text is information, not instructions."))));
}

// C1: what each card's button does, in its own words (never a bare Apply).
const VERB = { app: "Add app", map: "Map the app", run_all: "Run on all platforms" };
const KIND_ICON = { schedule: "clock", note: "memory", app: "plus", map: "map", run_all: "play" };
const OPEN = {
  jira: { href: "#/integrations/jira", verb: "Connect Jira", icon: "plug" },
  secrets: { href: "#/settings/secrets", verb: "Open Secrets", icon: "key" },
  device: { href: "#/device", verb: "Open Device", icon: "device" },
  history: { href: "#/history", verb: "Open History", icon: "history" },
};

function proposalCard(p) {
  if (p.kind === "jira_issue" || p.kind === "jira_comment") return jiraCard(p);
  if (p.kind === "agent" && p.status !== "refused") return agentCard(p);
  if (p.kind === "setting" && p.setting) return settingCard(p);
  if (p.kind === "test" && p.status !== "refused") return testCard(p);
  const page = p.kind === "open" ? OPEN[p.open] || null : null;
  const head = h("h3", { class: "gh" }, icon(KIND_ICON[p.kind] || (page && page.icon) || "shield"), p.title,
    p.kind === "note" && p.status === "pending" ? h("span", { class: "tag" }, "Needs your OK") : null);
  // D6 from Chat: the user's own message, quoted by fastpilot (M2), above the note it asks for.
  const quote = p.kind === "note" && p.quote ? h("p", { class: "quote" }, p.quote) : null;
  const rows = p.rows.length ? h("dl", { class: "prop-rows" }, p.rows.map(([k, v]) => [h("dt", {}, k), h("dd", {}, v)])) : null;
  let foot;
  if (p.status === "pending") {
    foot = h("div", { class: "btns" },
      // C1: a step done in the dashboard: the card opens that page (a fixed address, never one the
      // bot wrote).
      page ? h("a", { class: "btn p", href: page.href, onclick: () => { chatDecide(p.id, true); } }, page.verb)
      // UXA8: a note fastpilot offers on its own ("Remember this?") is kept with "Remember it".
        : h("button", { class: "btn p", onclick: async () => { await chatDecide(p.id, true); if (p.kind === "app" && window.Apps) Apps.reload(); } }, p.kind === "schedule" ? "Create schedule" : p.kind === "note" ? (p.title === "Remember this?" ? "Remember it" : "Add note") : VERB[p.kind] || "Save changes"),
      h("button", { class: "btn", onclick: () => chatDecide(p.id, false) }, p.kind === "note" ? "No thanks" : "Cancel"));
  } else if (p.status === "applied") {
    foot = h("div", { class: "prop-state ok", tabindex: "-1" }, "✓ ", rich(p.note || "Saved"));
  } else if (p.status === "cancelled") {
    foot = h("div", { class: "prop-state muted", tabindex: "-1" }, "Cancelled");
  } else {
    foot = h("div", { class: "prop-state bad", tabindex: "-1" }, rich(p.note || "Cannot be used"),
      p.kind === "limits" ? [" ", h("a", { class: "lk", href: "#/settings/limits" }, "Open Settings › Limits")] : null,
      p.kind === "note" ? [" ", h("a", { class: "lk", href: "#/memory" }, "Open Memory")] : null);
  }
  const note = p.status === "pending" && p.note ? h("p", { class: "s" }, rich(p.note)) : null;
  return h("section", { class: "grp prop " + p.status, "aria-label": p.title, "data-pid": String(p.id) }, head, h("div", { class: "prop-body" }, quote, rows, note, foot));
}

// D10c (S2): the bot's text with its "Techniques used" block drawn as a card: one heading per
// technique, its lines below, "✗" marked. Everything is set as text.
function botText(text) {
  const parts = String(text || "").split(/```[a-z]*\n?/);
  return parts.map((part, i) => {
    // QA B6: headings, **bold** and `code` of Claude's text, as DOM text.
    if (i % 2 === 0) return part.trim() ? h("div", { class: "bubble" }, markdown(part.trim())) : null;
    const lines = part.replace(/\s+$/, "").split("\n");
    if ((lines[0] || "").trim() !== "Techniques used") return h("pre", { class: "bubble code-block" }, part.trim());
    const blocks = [];
    for (const l of lines.slice(1)) {
      if (!l.trim()) continue;
      if (/^\S/.test(l)) blocks.push({ title: l.trim(), lines: [] });
      else if (blocks.length) blocks[blocks.length - 1].lines.push(l.trim());
    }
    // "Tried: a ✓ · b ✗ …" and "Rules covered: … (R1 ✓, R3 ✗ …)": one chip per value.
    const line = (x) => {
      const [label, rest] = x.includes(":") ? [x.slice(0, x.indexOf(":")), x.slice(x.indexOf(":") + 1).trim()] : ["", x];
      const items = rest.replace(/^[^(]*\((.*)\)$/, (_, inner) => inner).split(/ · |, /);
      if (/^(Tried|Rules covered|Transitions)$/i.test(label) && items.length > 1) {
        return h("div", { class: "tech-l" }, h("span", { class: "tl" }, label), h("span", { class: "tech-chips" }, items.map((it) => h("span", { class: "tchip" + (it.includes("✗") ? " no" : it.includes("not run") ? " nr" : " ok") }, it))));
      }
      return h("p", { class: "tech-l" }, label ? [h("b", {}, label + ": "), rest] : x);
    };
    return h("section", { class: "tech", "aria-label": "Techniques used" }, h("h3", {}, "Techniques used"),
      blocks.map((b) => h("div", { class: "tech-b" + (b.lines.some((x) => x.includes("✗")) ? " tech-bad" : "") }, h("b", {}, b.title),
        b.lines.some((x) => x.includes("✗")) ? h("span", { class: "tech-flag" }, "problem found") : null, b.lines.map(line))));
  });
}

// S8: the release verdict (read-only; the person decides). Fields come from fastpilot, text only.
const VERDICT_LOOK = {
  "SHIP": ["ok", "✓", "Main journeys and the regression pass."],
  "SHIP WITH FIXES": ["warn", "!", "Only minor problems; they are listed below."],
  "DO NOT SHIP": ["bad", "✗", "A main journey or the page itself fails."],
  "INCONCLUSIVE": ["unknown", "?", "Important evidence is missing. This is not a pass."],
};
// Copied text never carries control characters (Security, PR #100).
const plainText = (t) => String(t || "").replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u200b-\u200f\u202a-\u202e\u2066-\u2069]/g, " ");
// Markdown: text stays text in a tracker (no images, links or HTML from the page or the bot).
const mdText = (t) => plainText(t).replace(/[\\`*_[\]()<>!#|~]/g, (c) => "\\" + c).replace(/\n/g, " ");
async function copyText(text, what) {
  try { await navigator.clipboard.writeText(plainText(text)); toast(what + " copied", true); } catch (e) { toast("Could not copy: " + e.message); }
}
// PF4: the Speed row says it in plain words; the technical names are in a tooltip.
const SPEED_TERMS = [["Main content shown", "Main content shown = LCP (Largest Contentful Paint)"], ["Layout shift", "Layout shift = CLS (Cumulative Layout Shift)"], ["Loads in", "Loads in = time until the page's load event"]];
function speedTerms(detail) {
  return SPEED_TERMS.filter(([w]) => String(detail || "").includes(w)).map(([, d]) => d).join(" · ") || null;
}
function verdictCard(v) {
  const [cls, mark, fallback] = VERDICT_LOOK[v.verdict] || VERDICT_LOOK.INCONCLUSIVE;
  const rows = (v.evidence || []).map((e) => h("div", { class: "vrow" },
    h("span", { class: "vname" }, e.name), h("span", { class: "vdetail", title: speedTerms(e.detail) }, e.detail || ""),
    h("span", { class: "vstate " + e.state.replace(/ /g, "-") }, e.state)));
  const fixes = (v.fixes || []).length
    ? h("div", { class: "vfix" }, h("b", {}, "Fix before, or soon after, release"), h("ol", {}, v.fixes.map((f) => h("li", {}, f))))
    : null;
  const missing = (v.not_checked || []).length ? v.not_checked.join(" · ") : "nothing";
  const n = (v.fixes || []).length;
  return h("section", { class: "grp verdict " + cls, "aria-label": "Release check" },
    h("div", { class: "gh" }, icon("chat"), "Release check · the bot's summary"),
    h("div", { class: "prop-body" },
    v.title ? h("p", { class: "s vtitle" }, v.title) : null,
    // The card shows what the bot wrote; fastpilot did not check it (Security, PR #100).
    h("p", { class: "s muted" }, "Written by the bot from the checks it ran. fastpilot shows it as a card; it did not verify it."),
    h("div", { class: "vbanner", role: "status" }, h("span", { class: "vmark", "aria-hidden": "true" }, mark),
      h("div", {}, h("div", { class: "vword" }, v.verdict), h("div", { class: "vwhy" }, v.why || fallback))),
    rows.length ? h("div", { class: "vrows" }, rows) : null,
    fixes,
    h("div", { class: "vmissing" }, h("b", {}, "Not checked: "), missing),
    h("p", { class: "s" }, "This is a recommendation. You decide whether to release."),
    h("div", { class: "btns" },
      n ? h("button", { class: "btn p", onclick: () => chatSend(`Write bug reports for the ${n} problem${n > 1 ? "s" : ""} in this release check.`) }, `Write bug reports for the ${n} problem${n > 1 ? "s" : ""}`) : null,
      h("button", { class: "btn", onclick: () => copyText(v.text || "", "Report") }, "Copy as text"))));
}

// S8: a bug report preview. Not sent anywhere; Jira comes later.
function bugMarkdown(b) {
  const steps = (b.steps || []).map((s, i) => `${i + 1}. ${mdText(s)}`).join("\n");
  return `## ${mdText(b.title)}\n\n**Steps**\n${steps}\n\n**Expected:** ${mdText(b.expected)}\n\n**Actual:** ${mdText(b.actual)}\n\n**Reproduced:** ${mdText(b.reproduced || "not said")}\n\n**Environment:** ${mdText(b.environment || "not said")}\n\n**Evidence:** ${mdText(b.evidence || "none")}\n\n**Impact (estimate):** ${mdText(b.impact || "not measured")}\n`;
}
function bugCard(b) {
  const row = (k, v) => v ? [h("dt", {}, k), h("dd", {}, v)] : null;
  return h("section", { class: "grp bugcard", "aria-label": "Bug report preview" },
    h("div", { class: "gh" }, icon("alert"), "Bug report · preview", h("span", { class: "tag" }, "Not sent anywhere")),
    h("div", { class: "prop-body" }, h("dl", { class: "prop-rows" },
      row("Title", b.title),
      (b.steps || []).length ? [h("dt", {}, "Steps"), h("dd", {}, h("ol", {}, b.steps.map((s) => h("li", {}, s))))] : null,
      row("Expected", b.expected), row("Actual", b.actual), row("Reproduced", b.reproduced), row("Environment", b.environment),
      row("Evidence", b.evidence), row("Impact (estimate)", b.impact)),
    h("div", { class: "btns" },
      h("button", { class: "btn p", onclick: () => copyText(b.text || "", "Bug report") }, "Copy report"),
      h("button", { class: "btn", onclick: () => copyText(bugMarkdown(b), "Markdown") }, "Copy as Markdown"),
      // v0.6 scope: the dashboard only connects Jira; issues are filed by the bot or Claude Code.
      h("span", { class: "s jira-hint" }, "Ask the bot or Claude Code to file it in Jira"))));
}

function chatMsg(m) {
  if (m.role === "user") return h("li", { class: "msg me" }, h("div", { class: "bubble" }, m.text));
  const working = !m.done;
  return h("li", { class: "msg bot" },
    h("div", { class: "who" }, icon("mark", "sm"), "fastpilot bot"),
    m.tools && m.tools.length ? h("ul", { class: "tools", "aria-label": "What the bot did" }, m.tools.map((t) => h("li", {}, icon("play", "sm"), t))) : null,
    m.jira_reads && m.jira_reads.length ? jiraReads(m.jira_reads) : null,
    // M3: next to a card that loosens a limit, the bot's own words are folded away.
    m.text && (m.proposals || []).some((p) => p.setting && p.setting.level === "loosen")
      ? h("details", { class: "bot-said" }, h("summary", {}, "What the bot said"), h("div", { class: "bubble" }, markdown(m.text)))
      : m.text ? botText(m.text) : null,
    working ? h("div", { class: "typing" }, h("div", { class: "spin sm", "aria-hidden": "true" }), m.tools && m.tools.length ? " Working…" : " Thinking…") : null,
    m.error ? h("div", { class: "err", role: "alert" }, rich(m.error)) : null,
    m.verdict ? verdictCard(m.verdict) : null,
    m.bug ? bugCard(m.bug) : null,
    (m.proposals || []).map(proposalCard),
    m.page_attempts && m.page_attempts.length ? pageAttempts(m.page_attempts) : null);
}

let chatDraft = { agent: null, text: "", focused: false };
function renderChat() {
  const busy = !!(chatData && chatData.busy);
  const head = [h("span", { class: "pill" + (busy ? "" : " grey") }, busy ? "● the bot is working" : "● ready"),
    h("button", { class: "btn sm right", disabled: busy || !(chatData && chatData.messages.length), onclick: async () => {
      try { chatData = await api("/api/chat", { agent: chatAgent, reset: true }); } catch (e) { toast(e.message); }
      chatSig = "";
      renderChat();
    } }, icon("refresh", "sm"), "New conversation")];
  if (chatHost) {
    // Inside an agent's page: its own small bar instead of the page title.
    let bar = chatHost.querySelector(".chat-bar");
    if (!bar) { bar = h("div", { class: "chat-bar" }); chatHost.prepend(bar); }
    bar.replaceChildren(...head);
  } else {
    $top().replaceChildren(icon("chat"), h("h1", { tabindex: "-1" }, "Chat"), ...head);
  }
  let root = document.getElementById("chat-root");
  if (!root) {
    const input = h("textarea", { id: "chat-input", rows: "2", maxlength: "4000", placeholder: "Write to the fastpilot bot… for example: \"test the login in Chrome\"", "aria-label": "Message to the fastpilot bot" });
    // The page is drawn again when apps or settings load (Chat is the first screen): what is
    // being typed, and the cursor, stay.
    const kept = chatDraft.agent === chatAgent;
    if (kept) input.value = chatDraft.text;
    input.addEventListener("input", () => { chatDraft = { agent: chatAgent, text: input.value, focused: true }; });
    input.addEventListener("focus", () => { chatDraft = { agent: chatAgent, text: input.value, focused: true }; });
    // A box taken away by a redraw also blurs: only a box still on the page was left on purpose.
    input.addEventListener("blur", () => { setTimeout(() => { if (input.isConnected && chatDraft.agent === chatAgent) chatDraft.focused = false; }); });
    if (kept && chatDraft.focused) queueMicrotask(() => { if (input.isConnected) { input.focus(); input.setSelectionRange(input.value.length, input.value.length); } });
    input.addEventListener("keydown", (e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); chatSend(input.value); } });
    root = h("div", { id: "chat-root", class: "chat" },
      h("ul", { id: "chat-log", class: "log", role: "log", "aria-live": "polite", "aria-label": "Conversation" }),
      h("div", { class: "composer" }, input, h("button", { id: "chat-send", class: "btn p", onclick: () => chatSend(input.value) }, icon("send", "sm"), "Send")),
      // DC6: one line.
      h("p", { class: "s muted chat-foot" }, "Changes wait for your click on their card. Passwords are never typed here."));
    if (chatHost) chatHost.append(root); else $body().replaceChildren(root);
  }
  const box = document.getElementById("chat-input");
  if (chatPrefill && box) { box.value = chatPrefill; chatPrefill = null; box.focus(); }
  const send = document.getElementById("chat-send");
  if (send) send.disabled = busy;
  const log = document.getElementById("chat-log");
  const msgs = (chatData && chatData.messages) || [];
  const sig = JSON.stringify(msgs) + (chatData && chatData.error);
  if (sig === chatSig) return;
  chatSig = sig;
  const atBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 40;
  if (chatData && chatData.error) {
    log.replaceChildren(h("li", { class: "err", role: "alert" }, "Cannot read the chat: " + chatData.error));
  } else if (!msgs.length) {
    // DC1: three one-click suggestions; fixed sentences written by fastpilot, sent at once.
    log.replaceChildren(h("li", { class: "empty" }, icon("chat", "big"), h("b", {}, "Ask or tell the fastpilot bot"),
      h("span", {}, "Run tests, ask why one failed, make a schedule, or make limits stricter."),
      h("div", { class: "chat-suggest" }, CHAT_SUGGESTIONS.map((t) => h("button", { type: "button", class: "say-row", onclick: () => chatSend(t) }, h("span", {}, t), h("span", { class: "arrow", "aria-hidden": "true" }, "→"))))));
  } else {
    log.replaceChildren(...msgs.map(chatMsg));
  }
  if (atBottom || msgs.length) log.scrollTop = log.scrollHeight;
}


// ---- Schedules (C3, D5) ----
let schedData = null, schedBusy = false, schedForm = null; // schedForm: null | { id?, ... }
let schedMade = null; // S5: the last preset click, for "Created · Undo": { id, new, text }
let schedMoreOpen = false; // More options stays open after the form closes, with focus on "New schedule"
const DAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const KINDS = [["day", "Fixed time"], ["hours", "Every N hours"], ["weekday", "Weekdays"], ["week", "Weekly"], ["release", "Before each release"], ["minutes", "Release watch"]];

let schedTimer = null;
async function loadSchedules() {
  schedBusy = true;
  try { schedData = await api("/api/schedules"); } catch (e) { schedData = { error: e.message, schedules: [] }; }
  schedBusy = false;
  const here = (location.hash || "").startsWith("#/schedules");
  // An open form keeps what is typed: the list refreshes after it closes.
  if (here && !schedForm) pageSchedules();
  // A run in progress: look again until it has finished.
  clearTimeout(schedTimer);
  if (here && (schedData.schedules || []).some((x) => x.running)) schedTimer = setTimeout(loadSchedules, 3000);
}

async function schedAction(body, done) {
  try {
    const r = await api("/api/schedules", body);
    clearToast();
    if (done) done(r);
    await loadSchedules();
    return r;
  } catch (e) {
    toast(e.message);
    return null;
  }
}

const pad = (n) => String(n).padStart(2, "0");
const hm = (d) => `${pad(d.getHours())}:${pad(d.getMinutes())}`;

// "14:00" today, "tomorrow 02:00", "Mon 09:00" this week, else "24 Nov 09:00".
function when(d) {
  const now = new Date();
  const day = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(d) - day(now)) / 86400000);
  if (diff === 0) return hm(d);
  if (diff === 1) return `tomorrow ${hm(d)}`;
  if (diff === -1) return `yesterday ${hm(d)}`;
  if (diff > 1 && diff < 7) return `${d.toLocaleDateString("en-GB", { weekday: "short" })} ${hm(d)}`;
  return `${d.toLocaleDateString("en-GB", { day: "numeric", month: "short" })} ${hm(d)}`;
}

// The next run on this computer's clock, from the schedule's times of day and days.
function nextRun(x) {
  if (!x.schedule.enabled) return null;
  const now = new Date();
  for (let i = 0; i < 8; i++) {
    const d = new Date(now.getFullYear(), now.getMonth(), now.getDate() + i);
    const dow = d.getDay() === 0 ? 7 : d.getDay();
    if (x.days && !x.days.includes(dow)) continue;
    for (const t of x.slots || []) {
      const [hh, mm] = t.split(":").map(Number);
      const at = new Date(d.getFullYear(), d.getMonth(), d.getDate(), hh, mm);
      if (at > now) return at;
    }
  }
  return null;
}

function lastRun(x) {
  const l = x.last;
  if (x.running) return h("span", { class: "run" }, h("div", { class: "spin sm", "aria-hidden": "true" }), " Running…");
  if (!l || (l.passed === undefined || l.passed === null)) {
    return l && !x.running && l.started_ms ? h("span", { class: "bad" }, "Did not finish · ", when(new Date(l.started_ms)), h("div", { class: "s" }, "The run stopped before it finished (was the computer asleep or off?)."))
      : h("span", { class: "muted" }, "Not run yet");
  }
  const at = when(new Date(l.finished_ms || l.started_ms));
  // V042 (FastUI/UX D1): a secret that is not set is a setup problem: amber "could not run", never a pass or a failure.
  if (l.needs) {
    return h("div", {}, h("span", { class: "warn-txt" }, "Could not run · ", at),
      h("div", { class: "s" }, `${x.schedule.name} could not run. It needs the secret ${l.needs}, and no value is set where schedules run. Nothing ran, so this is not a pass.`),
      h("div", { class: "s" }, h("a", { class: "lk", href: "#/settings/secrets" }, "Open Settings → Secrets")),
      l.notified && l.notified.length ? h("div", { class: "s" }, l.notified.join(" · ")) : null);
  }
  const count = l.total !== undefined && l.total !== null ? `${l.ok}/${l.total}` : (l.passed ? "passed" : "failed");
  const label = [l.passed ? "✓ " : "✗ ", count, " ", at];
  const sr = `${l.passed ? "Passed" : "Failed"} ${count} at ${at}, open the report`;
  const res = l.report
    ? h("a", { class: "lk " + (l.passed ? "ok" : "bad"), href: `/api/schedules/report?id=${encodeURIComponent(x.schedule.id)}`, target: "_blank", rel: "noopener", "aria-label": sr }, ...label)
    : h("span", { class: l.passed ? "ok" : "bad" }, ...label);
  return h("div", {}, res,
    (!l.passed || x.schedule.watch) && l.summary ? h("div", { class: "s" }, l.summary) : null,
    l.notified && l.notified.length ? h("div", { class: "s" }, l.notified.join(" · ")) : null);
}

function notifyText(x) {
  const n = x.schedule.notify || {};
  const out = [];
  if (n.webhook) out.push(x.webhook ? `Slack (${x.webhook.replace(/^https?:\/\//, "").replace(/\/•••$/, "")})` : "Slack (URL missing)");
  if (n.os) out.push("Mac notification");
  if (n.github) out.push(`GitHub issue in ${n.github}`);
  return out.length ? out.join(", ") : h("span", { class: "muted" }, "None");
}

function schedRow(x) {
  const s = x.schedule;
  const on = h("button", { class: "tg", role: "switch", "aria-checked": String(!!s.enabled), "aria-label": `Schedule ${s.name} on`,
    onclick: () => schedAction({ action: s.enabled ? "disable" : "enable", id: s.id }) });
  const next = nextRun(x);
  return h("tr", { "data-id": s.id },
    h("td", {}, h("a", { class: "lk", href: `#/agents/${encodeURIComponent(s.agent || "default")}/schedules` }, x.agent_name || "")),
    h("td", {}, h("b", { class: "row-name", tabindex: "-1" }, s.name), h("div", { class: "s" }, x.tests_text)),
    h("td", {}, x.when_text, x.watch_text ? h("div", { class: "s" }, x.watch_text) : null),
    h("td", { class: "next" }, s.enabled ? (next ? when(next) : s.watch ? `in ${s.when.minutes} min or less` : "—") : h("span", { class: "muted" }, "Off")),
    h("td", { class: "last" }, lastRun(x)),
    h("td", {}, notifyText(x)),
    h("td", {}, on),
    h("td", {}, h("div", { class: "acts" },
      h("button", { class: "btn sm", disabled: x.running, onclick: () => schedAction({ action: "run", id: s.id }, () => toast(`"${s.name}" is running now. The result shows under Last.`, true)).then(() => setTimeout(loadSchedules, 1500)) }, icon("play", "sm"), "Run now"),
      h("button", { class: "btn sm", onclick: () => { schedForm = Object.assign({}, s, { webhook_set: !!x.webhook }); pageSchedules(); document.querySelector(".sched-form input, .sched-form select")?.focus(); } }, "Edit"),
      s.notify && (s.notify.webhook || s.notify.os || s.notify.github) ? h("button", { class: "btn sm", onclick: () => schedAction({ action: "test", id: s.id }, (r) => toast("Test notification: " + r.sent.join(" · "), !r.sent.some((x) => x.includes("failed")))) }, "Test notification") : null,
      h("button", { class: "btn sm danger", onclick: async () => { if (await UI.confirmDialog({ title: `Delete the schedule "${s.name}"?`, text: "It stops running, and its entry in the system scheduler is removed too. Your tests stay.", verb: "Delete schedule" })) schedAction({ action: "delete", id: s.id }); } }, "Delete"))));
}

function schedPanel() {
  const f = schedForm;
  const editing = !!f.id;
  const w = f.when || {};
  // Each field has its own error line; a refused save points at the field it is about.
  const errs = {};
  const field = (label, input, hint, key, cls) => {
    const err = h("span", { class: "fld-err", role: "alert", hidden: true, id: key ? `err-${key}` : undefined });
    if (key) {
      errs[key] = { err, input };
      input.setAttribute("aria-describedby", `err-${key}`);
    }
    return h("label", { class: "fld" + (cls ? " " + cls : "") }, h("span", { class: "l" }, label), input, hint ? h("span", { class: "s" }, hint) : null, err);
  };
  // A1: tests are picked from this app's saved tests (all of them first); nobody types a path.
  // A schedule saved earlier with another path keeps it, shown as it is.
  const found = (schedData && schedData.tests) || [];
  const all = schedData && schedData.tests_hint;
  const options = [];
  if (all) options.push([all, found.length === 1 ? "The 1 test of this app" : `All ${found.length} tests of this app`]);
  found.forEach((t) => options.push([t.path, t.title]));
  if (f.tests && !options.some(([v]) => v === f.tests)) options.push([f.tests, `${f.tests} (as saved)`]);
  const pick = h("select", { class: "sel", "aria-label": "Which tests" }, options.map(([v, t]) => h("option", { value: v, selected: f.tests ? v === f.tests : false }, t)));
  const noTests = !options.length ? UI.notice("info", "This app has no saved tests yet. Save a test from Chat first, then schedule it.") : null;
  // Kind: a radio group shown as a segmented control.
  let kind = w.every || "day";
  const seg = h("div", { class: "seg", role: "radiogroup", "aria-label": "How often" });
  const time = h("input", { type: "time", value: w.time || "08:00" });
  const hours = h("input", { type: "number", min: "1", max: "24", step: "1", value: String(w.hours || 2), class: "num", "aria-label": "Hours between runs" });
  const from = h("input", { type: "time", value: w.from || "", "aria-label": "Active from" });
  const to = h("input", { type: "time", value: w.to || "", "aria-label": "Active until" });
  const day = h("select", { class: "sel" }, DAYS.map((d, i) => h("option", { value: String(i + 1), selected: w.day === i + 1 }, d)));
  // R1: a release watch runs every N minutes for M hours, then ends by itself.
  const mins = h("select", { class: "sel", "aria-label": "Minutes between runs" }, [5, 10, 15, 20, 30, 60].map((m) => h("option", { value: String(m), selected: (w.minutes || 10) === m }, `${m} minutes`)));
  const left = f.watch && f.watch.until_ms ? Math.max(1, Math.round((f.watch.until_ms - Date.now()) / 3600000)) : 2;
  const forH = h("input", { type: "number", min: "1", max: "24", step: "1", value: String(Math.min(24, left)), class: "num", "aria-label": "Hours to watch" });
  const watchWrap = h("div", { class: "fld" }, h("span", { class: "l" }, "Every"),
    h("div", { class: "inline" }, mins, h("span", {}, "for"), forH, h("span", {}, "hours")),
    h("span", { class: "s" }, "The first run is the \"before\". After that you hear only about a problem that is new since then, once. It ends by itself."),
    h("span", { class: "fld-err", role: "alert", hidden: true, id: "err-watch" }));
  errs.watch = { err: watchWrap.querySelector(".fld-err"), input: forH };
  forH.setAttribute("aria-describedby", "err-watch");
  const timeWrap = field("At (this computer's time)", time, null, "time");
  const hoursWrap = h("div", { class: "fld" }, h("span", { class: "l" }, "Every"),
    h("div", { class: "inline" }, hours, h("span", {}, "hours")), h("span", { class: "fld-err", role: "alert", hidden: true, id: "err-hours" }));
  errs.hours = { err: hoursWrap.querySelector(".fld-err"), input: hours };
  hours.setAttribute("aria-describedby", "err-hours");
  const windowWrap = h("div", { class: "fld" }, h("span", { class: "l" }, "Active (optional)"),
    h("div", { class: "inline" }, from, h("span", {}, "–"), to), h("span", { class: "s" }, "Empty: all day, from 00:00."),
    h("span", { class: "fld-err", role: "alert", hidden: true, id: "err-window" }));
  errs.window = { err: windowWrap.querySelector(".fld-err"), input: from };
  from.setAttribute("aria-describedby", "err-window");
  const dayWrap = field("Day", day, null, "day");
  const sync = () => {
    seg.querySelectorAll("button").forEach((b) => { const on = b.dataset.v === kind; b.setAttribute("aria-checked", String(on)); b.tabIndex = on ? 0 : -1; });
    timeWrap.hidden = kind === "hours" || kind === "minutes" || kind === "release";
    releaseHint.hidden = kind !== "release";
    hoursWrap.hidden = windowWrap.hidden = kind !== "hours";
    dayWrap.hidden = kind !== "week";
    watchWrap.hidden = kind !== "minutes";
    notifyLegend.textContent = kind === "minutes" ? "Notify me about a new problem (any you like)" : "Notify me when it fails (any you like)";
  };
  const notifyLegend = h("legend", { class: "l" }, "Notify me when it fails (any you like)");
  const releaseHint = h("p", { class: "s" }, "No time: it runs when you ask Chat \u201cCan we release?\u201d, or when you press Run now.");
  KINDS.forEach(([v, t]) => seg.append(h("button", { type: "button", role: "radio", "data-v": v, onclick: () => { kind = v; sync(); },
    onkeydown: (e) => {
      const i = KINDS.findIndex(([k]) => k === kind);
      const step = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
      if (!step) return;
      e.preventDefault();
      kind = KINDS[(i + step + KINDS.length) % KINDS.length][0];
      sync();
      seg.querySelector(`[data-v="${kind}"]`).focus();
    } }, t)));
  sync();
  const n = f.notify || {};
  const wh = h("input", { type: "checkbox", checked: !!n.webhook });
  const whUrl = h("input", { type: "password", autocomplete: "off", placeholder: f.webhook_set ? "saved · type to replace it" : "https://hooks.slack.com/services/…", class: "mono" });
  // S5: a Mac notification by default; the GitHub repo comes from the app's own repo.
  const os = h("input", { type: "checkbox", checked: editing ? !!n.os : true });
  const gh = h("input", { type: "checkbox", checked: !!n.github });
  const repo = h("input", { type: "text", value: n.github || (schedData && schedData.github_repo) || "", placeholder: "owner/repo", class: "mono" });
  const whField = field("Webhook URL", whUrl, "Kept as a secret on this computer and not shown again.", "webhook");
  const repoField = field("GitHub repo", repo, h("span", {}, "Written owner/repo. Token from GITHUB_TOKEN / GH_TOKEN, or the Keychain item ", code("fastpilot-github"), "."), "repo");
  const syncNotify = () => { whField.hidden = !wh.checked; repoField.hidden = !gh.checked; };
  wh.addEventListener("change", syncNotify);
  gh.addEventListener("change", syncNotify);
  syncNotify();
  const where = (msg) => {
    const m = msg.toLowerCase();
    if (m.includes("name") && !m.includes("owner/name")) return "name";
    if (m.includes("release watch") || m.includes("minutes")) return "watch";
    if (m.includes("every n hours") || m.includes("n must")) return "hours";
    if (m.includes("window")) return "window";
    if (m.includes("tests not found") || m.includes(".md")) return "tests";
    if (m.includes("time")) return "time";
    if (m.includes("day")) return "day";
    if (m.includes("repo")) return "repo";
    if (m.includes("webhook")) return "webhook";
    return null;
  };
  const showError = (msg) => {
    Object.values(errs).forEach(({ err, input }) => { err.hidden = true; input.removeAttribute("aria-invalid"); });
    const at = errs[where(msg)];
    if (!at || at.err.parentElement.closest("[hidden]") || at.input.closest("[hidden]")) { toast(msg); return; }
    at.err.textContent = msg;
    at.err.hidden = false;
    at.input.setAttribute("aria-invalid", "true");
    at.input.focus();
  };
  const cancel = () => { schedForm = null; schedMoreOpen = true; clearToast(); pageSchedules(); document.querySelector(".more-body .btn")?.focus(); };
  const save = async () => {
    const whenOut = { every: kind };
    if (kind === "minutes") {
      whenOut.minutes = Number(mins.value);
    } else if (kind === "hours") {
      whenOut.hours = Number(hours.value);
      if (from.value || to.value) { whenOut.from = from.value || undefined; whenOut.to = to.value || undefined; }
    } else if (kind !== "release") {
      whenOut.time = time.value;
      if (kind === "week") whenOut.day = Number(day.value);
    }
    // A1: the name is made by fastpilot from what runs and when; an unchanged edit keeps it.
    const same = editing && pick.value === f.tests && JSON.stringify(whenOut) === JSON.stringify(Object.fromEntries(Object.entries(f.when || {}).filter(([, v]) => v !== null && v !== "" && v !== undefined)));
    const schedule = {
      id: f.id || "", name: same ? f.name : "", tests: pick.value, enabled: f.enabled !== false, when: whenOut, agent: f.agent || undefined,
      notify: { webhook: wh.checked, os: os.checked, github: gh.checked ? repo.value.trim() : undefined },
    };
    const body = { action: "save", schedule };
    if (kind === "minutes") body.watch_hours = Number(forH.value);
    if (whUrl.value.trim()) body.webhook = whUrl.value.trim();
    let r;
    try { r = await api("/api/schedules", body); } catch (e) { showError(e.message); return; }
    schedForm = null;
    await loadSchedules();
    toast(r.watch ? "Release watch started: the first run, the \"before\", is running now." : editing ? "Schedule saved." : "Schedule created. It runs even when this page is closed.", true);
    // Focus on the saved schedule in the list (the form and its button are gone).
    document.querySelector(`tr[data-id="${r.id}"] .row-name`)?.focus();
  };
  return h("section", { class: "grp sched-form", "aria-label": editing ? "Edit schedule" : "New schedule",
    onkeydown: (e) => { if (e.key === "Escape") { e.preventDefault(); cancel(); } } },
    h("h2", { class: "gh" }, icon("clock"), editing ? `edit schedule · ${f.name}` : "new schedule"),
    h("div", { class: "form" },
      noTests,
      field("Tests", pick, null, "tests"),
      h("div", { class: "fld" }, h("span", { class: "l", id: "kind-l" }, "How often"), seg),
      h("div", { class: "row2" }, timeWrap, hoursWrap, windowWrap, dayWrap, watchWrap),
      releaseHint,
      h("fieldset", { class: "fld" }, notifyLegend,
        h("label", { class: "ck" }, os, " Mac notification"),
        h("label", { class: "ck" }, wh, " Slack (webhook)"), h("div", { class: "sub-fld" }, whField),
        h("label", { class: "ck" }, gh, " GitHub issue"), h("div", { class: "sub-fld" }, repoField)),
      h("div", { class: "btns" },
        h("button", { class: "btn", onclick: cancel }, "Cancel"),
        h("button", { class: "btn p", onclick: save }, editing ? "Save changes" : "Create schedule")),
      h("p", { class: "s muted" }, "The name is made from the tests and the time. Or say it in Chat: \"run the checkout tests every 2 hours\". Escape closes this without saving.")));
}

// S5: the three one-click schedules for all of this app's tests. A preset that is on says
// "On · Turn off" (a second click cannot make it twice); a click is answered "Created · Undo".
function schedPresets() {
  const list = schedData.presets || [];
  const n = (schedData.tests || []).length;
  if (!list.length || !n) return null;
  const turnOn = async (p) => {
    const r = await schedAction({ action: "preset", preset: p.key });
    if (!r) return;
    schedMade = r;
    pageSchedules();
    document.querySelector(".sched-made button")?.focus();
  };
  const cards = list.map((p) => h("div", { class: "preset" + (p.on ? " on" : ""), "data-preset": p.key },
    h("div", { class: "preset-top" }, h("b", {}, p.title), p.on ? h("span", { class: "pill ok" }, "On") : null),
    h("div", { class: "s" }, p.sub),
    p.on ? h("button", { class: "lk", onclick: () => { schedMade = null; schedAction({ action: "disable", id: p.id }); } }, "Turn off")
      : h("button", { class: "btn sm", onclick: () => turnOn(p) }, "Turn on")));
  const made = schedMade ? h("div", { class: "sched-made" }, UI.notice("ok", schedMade.text, " ",
    h("button", { class: "lk", onclick: async () => {
      const m = schedMade;
      schedMade = null;
      await schedAction(m.new ? { action: "delete", id: m.id } : { action: "disable", id: m.id }, () => toast("Undone.", true));
    } }, "Undo"))) : null;
  return h("section", { class: "grp sched-presets", "aria-label": "Run your tests by themselves" },
    h("div", { class: "prop-body" },
      h("h2", {}, `Run all ${n === 1 ? "1 test" : n + " tests"} of this app by themselves`),
      h("p", { class: "s" }, "Pick one. fastpilot runs them and tells you on this Mac only when something breaks."),
      h("div", { class: "presets" }, cards),
      made));
}

// S7: before and after a release, for this app: one button checks it (Chat "Can we release?",
// on the app's own address), one toggle watches it for 2 hours afterwards.
function schedRelease() {
  if (!(schedData.tests || []).length) return null;
  const app = schedData.app || {};
  const w = schedData.watching;
  const check = h("button", { class: "btn sm p", onclick: async () => {
    await openChat("Can we release?");
  } }, "Check before release");
  const tg = h("button", { class: "tg", role: "switch", "aria-checked": String(!!w), "aria-label": "Watch for 2 hours after a release",
    onclick: () => schedAction({ action: "watch", on: !w }, (r) => toast(r.on ? "Watching for 2 hours: the first run, the \"before\", is running now." : "The release watch is off.", true)) });
  return h("section", { class: "grp sched-release", "aria-label": "Before and after a release" },
    h("div", { class: "prop-body" },
      h("h2", {}, "Before and after a release"),
      h("div", { class: "rel-row" },
        // L07-1: the row says what happens; the button is the action.
        h("div", {}, h("b", {}, "Tests and a ship check"),
          h("div", { class: "s" }, app.web ? `Runs the tests and a ship check of ${app.name ? app.name + " at " : ""}${app.web}, then says Ship or Do not ship.` : "Runs the tests and a ship check, then says Ship or Do not ship.")),
        check),
      h("div", { class: "rel-row" },
        h("div", {}, h("b", {}, "Watch for 2 hours after a release"),
          h("div", { class: "s" }, w && w.until_ms ? `On until ${hm(new Date(w.until_ms))}. Every 10 minutes; you hear only about a problem that is new since the release.` : "Every 10 minutes; you hear only about a problem that is new since the release. It ends by itself.")),
        tg)));
}

function pageSchedules() {
  const openForm = () => { schedForm = {}; pageSchedules(); document.querySelector(".sched-form select, .sched-form input")?.focus(); };
  UI.pageHeader({ icon: "clock", title: "Schedules", sub: "Run your tests by themselves" });
  if (!schedData) {
    $body().replaceChildren(h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" }), h("b", {}, "Reading schedules…")));
    if (!schedBusy) loadSchedules();
    return;
  }
  const list = schedData.schedules || [];
  const table = list.length
    ? h("div", { class: "grp" }, h("div", { class: "gh" }, icon("clock"), `schedules · ${list.length} · they run even when this page is closed`),
      h("div", { class: "tbl-wrap" }, h("table", { class: "tbl" },
        h("thead", {}, h("tr", {}, ["Agent", "Name", "When", "Next", "Last", "Notify", "On", ""].map((t) => h("th", {}, t)))),
        h("tbody", {}, list.map(schedRow)))))
    // L07-1: one way in. With saved tests the presets above (and More options) make a schedule, so
    // the empty list has no button of its own.
    : h("div", { class: "grp" }, h("div", { class: "gh" }, "schedules · empty"),
      UI.empty({ icon: "clock", title: "Nothing is scheduled yet",
        text: "Turn on one of the buttons above, or say it in Chat: \"run the checkout tests every 2 hours\". You get a message only when something fails." }));
  // L07-1: no saved test yet: nothing to schedule, so no form with an empty list; one way to Chat.
  if (!(schedData.tests || []).length) {
    const first = h("div", { class: "grp sched-first" },
      UI.empty({ icon: "tests", title: "No saved test yet",
        text: "A schedule runs your saved tests by themselves. Make your first test in Chat, then come back here.",
        action: { label: "Make a test first in Chat", icon: "chat", onclick: () => { location.hash = "#/agents"; } } }));
    $body().replaceChildren(...UI.kids(schedData.error ? UI.notice("error", "Could not read the schedules: " + schedData.error) : null, first, list.length ? table : null));
    return;
  }
  // A1: the full form (which tests, other times, Slack and GitHub) is under More options.
  const more = UI.moreOptions({ label: "More options", open: !!schedForm || schedMoreOpen },
    h("p", { class: "s muted" }, "Choose which tests, other times, Slack and GitHub messages."),
    schedForm ? schedPanel() : h("button", { class: "btn sm", onclick: openForm }, icon("plus", "sm"), "New schedule"));
  $body().replaceChildren(...UI.kids(schedData.error ? UI.notice("error", "Could not read the schedules: " + schedData.error) : null, schedPresets(), schedRelease(), more, table,
    sayIt(["Run all tests every 2 hours", "Turn off the nightly run"])));
}

// ---- router ----
// Pages in their own files (history.js, …) register here: PAGES[name] = (sub) => …
const PAGES = {};
function render() {
  if (!S) return;
  const hash = location.hash || HOME;
  // E6: "Apps" (Jira and the rest) is now Integrations; old links still land there.
  if (hash === "#/apps" || hash.startsWith("#/apps/jira")) { location.replace(hash.replace("#/apps", "#/integrations")); return; }
  const [, page, sub] = hash.split("/");
  if (window.Apps) Apps.bar();
  if (page === "apps" && (sub || "").startsWith("add")) {
    pageSettings("apps");
    Apps.route();
    return;
  }
  if (page !== "live") stopLive();
  // E4: the simple menu until the first saved test.
  if (window.Welcome) Welcome.menu();
  document.querySelectorAll(".rail a").forEach((a) => {
    const on = a.dataset.nav === page;
    a.classList.toggle("on", on);
    if (on) a.setAttribute("aria-current", "page"); else a.removeAttribute("aria-current");
  });
  // E4: the first screen is the first-run wizard until a test is saved (or it is skipped).
  if (window.Welcome && Welcome.wants(page)) {
    Welcome.page();
    return;
  }
  if (page === "live") {
    if (!liveTimer) {
      $top().replaceChildren(icon("live"), h("h1", { tabindex: "-1" }, "Live"));
      $body().replaceChildren(h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" }), h("b", {}, "Reading sessions…")));
      pollLive();
    }
  } else if (page === "chat") {
    // Links made before U1: the one Chat page.
    location.replace("#/agents/default/chat");
  } else if (page === "schedules") {
    if (!schedData) loadSchedules(); else pageSchedules();
  } else if (page === "device") {
    if (!doctorCache && !doctorBusy) loadDoctor(false);
    pageDevice();
  } else if (PAGES[page]) {
    PAGES[page](sub);
  } else {
    pageSettings(sub);
  }
}

window.addEventListener("hashchange", async () => {
  // Settings are read again on the way in: the file may have changed (or broken) meanwhile.
  if (location.hash.startsWith("#/settings")) {
    try { await loadSettings(); } catch (e) { toast("Could not load the settings: " + e.message); }
  }
  render();
  document.getElementById("main").focus({ preventScroll: true });
});
window.addEventListener("DOMContentLoaded", async () => {
  try {
    await loadSettings();
    render();
    askRemember();
  } catch (e) {
    $top().replaceChildren("fastpilot");
    $body().replaceChildren(h("div", { class: "err", role: "alert" }, "Cannot load the settings: " + e.message));
  }
});
