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
    throw new Error("the fastpilot ui server is not answering; run `fastpilot ui` again in Terminal");
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
function currentPage() {
  return (location.hash || "#/live").split("/").slice(1, 3).join("/");
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
  return h("nav", { class: "snav", "aria-label": "Settings" }, item("model", "AI model"), item("limits", "Limits & permissions"), item("secret", "Secret"));
}

function topSettings() {
  $top().replaceChildren(icon("settings"), h("h1", { tabindex: "-1" }, "Settings"));
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
  if (provData.error) return h("div", { class: "err", role: "alert" }, "Cannot read the provider: " + provData.error);
  const preset0 = (n) => provData.presets.find((p) => p.name === n) || {};
  const d = provDraft || { provider: provData.provider, base_url: provData.base_url || preset0(provData.provider).base_url || "", model: provData.model || "" };
  const pick = h("select", { class: "sel", "aria-label": "Model provider for fastpilot run", disabled: provData.env.provider },
    provData.presets.map((p) => h("option", { value: p.name, selected: p.name === d.provider }, PROV_LABEL[p.name] || p.name)));
  const preset = (n) => provData.presets.find((p) => p.name === n) || {};
  const hosted = (n) => ["openai", "openrouter", "groq"].includes(n);
  const url = h("input", { type: "text", class: "mono", value: d.base_url, placeholder: "https://…/v1", "aria-label": "Base URL" });
  const urlHint = h("span", { class: "s" });
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
    h("button", { class: "btn p", disabled: provBusy || provData.env.provider, onclick: () => provAction({ action: "save", provider: pick.value, base_url: url.value, model: model.value, key: key.value }) }, "Save"),
    provData.provider !== "claude" ? h("button", { class: "btn", disabled: provBusy, onclick: () => provAction({ action: "test" }) }, provBusy ? "Testing…" : "Test the connection") : null,
    provData.provider !== "claude" && k.has ? h("button", { class: "btn danger", disabled: provBusy, onclick: () => { if (confirm("Delete the saved API key?")) provAction({ action: "delete-key" }); } }, "Delete the key") : null);
  return h("div", { class: "grp" },
    h("div", { class: "gh" }, icon("plug"), "who runs the tests · fastpilot run"),
    provData.env.provider ? h("div", { class: "banner" }, icon("info"), h("span", {}, "The provider is set in the environment (FASTPILOT_PROVIDER…), which wins over this setting.")) : null,
    opt("Provider", "Claude Code uses your login. An OpenAI-compatible API is optional; it applies from the next run.", pick, null),
    h("div", { class: "prov" }, detail, btns,
      provResult ? h("p", { class: "prov-result " + (provResult.ok ? "ok" : "bad"), role: "status" }, provResult.ok ? "✓ " : "✗ ", provResult.text) : null));
}

function pageModel() {
  const run = doctorCache && doctorCache.checks.find((c) => c.area === "run");
  const status = !doctorCache ? h("span", { class: "tag" }, "Checking…")
    : run && run.status === "ok" ? h("span", { class: "tag" }, "✓ Installed")
    : h("span", { class: "tag warn" }, "Not installed");
  const replay = [["", `Built in · ${MODELS.find(([v]) => v === S.defaults.model_replay)?.[1] || S.defaults.model_replay}`], ...MODELS];
  const explore = [["", `Built in · ${MODELS.find(([v]) => v === S.defaults.model_explore)?.[1] || S.defaults.model_explore}`], ...MODELS];
  return h("section", { class: "panel" },
    h("h2", {}, "AI model", savedNote()),
    h("p", { class: "sub" }, "The model is only used when fastpilot runs tests itself (", code("fastpilot run"), "). From Claude Code or Cursor, the model is chosen in that app."),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("model"), "ai model · who runs what"),
      opt("Replay (fast & cheap)", "Replays steps that are already recorded", select("model_replay", replay, "Replay model"), get("model_replay")),
      opt("Exploring / repairing", "Used when a replay fails, or for a new test", select("model_explore", explore, "Exploring model"), get("model_explore")),
      opt("Claude Code", run ? `${run.detail}` : "Uses your Claude Code login, no API key", status)),
    providerGroup());
}

const NEW_032 = "fp-new-0.3.2-limits";
function pageLimits() {
  const confirmShow = (w) => (w === "default" ? "default (delete, pay, buy, …)" : w);
  const emptied = (key) => { const s = get(key); return s && s.source !== "default" && Array.isArray(s.value) && !s.value.length; };
  let seen = false;
  try { seen = localStorage.getItem(NEW_032) === "1"; } catch (e) { /* private mode: show it */ }
  const news = seen ? null : h("div", { class: "banner info new" }, icon("info"),
    h("span", {}, h("b", {}, "New in 0.3.2: "), "payment and account-deletion buttons are now blocked by default. If you do want to test a checkout, remove the word below or click ", h("b", {}, "Empty it"), "."),
    h("button", { class: "rm", "aria-label": "Close this notice", onclick: () => { try { localStorage.setItem(NEW_032, "1"); } catch (e) { /* ignore */ } render(); } }, icon("close", "sm")));
  return h("section", { class: "panel" },
    h("h2", {}, "Limits & permissions", savedNote()),
    h("p", { class: "sub" }, "These rules apply to every test: from Claude Code, Cursor, the CLI, or ", code("fastpilot run"), "."),
    news,
    h("div", { class: "banner" }, icon("info"), h("span", {}, "Changes apply from the next run. A session that is running keeps the rules it started with. Through chat, the AI can only make rules stricter, never looser. Saved in ", code(S.path), ".")),
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
      opt("Longest time per test", "The test stops and fails when it goes over",
        select("test_timeout", [["", "30 minutes (built in)"], ["5", "5 minutes"], ["10", "10 minutes"], ["15", "15 minutes"], ["60", "60 minutes"]], "Longest time"), get("test_timeout"))));
}

function pageSecret() {
  return h("section", { class: "panel" },
    h("h2", {}, "Secret", savedNote()),
    h("p", { class: "sub" }, "Variables a flow may read as ", code("${NAME}"), ". Their values always show as *** in output."),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("key"), "secret"),
      opt("Secret name", "Only the name is saved here; this page never reads or shows the value.",
        chips("vars", { placeholder: "for example DEMO_PASS", addLabel: "Add a secret name", empty: "None yet" }), get("vars")),
      h("div", { class: "opt" }, h("div", {}, h("div", { class: "l" }, "Where is the value?"),
        h("div", { class: "d" }, "Put the value in the MCP configuration, for example ", code("claude mcp add fastpilot -e DEMO_PASS=… -- fastpilot mcp"), ", or in the environment when you run fastpilot.")))));
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
  topSettings();
  if (S.error) {
    $body().replaceChildren(h("div", { class: "set" }, settingsNav(sub === "model" || sub === "secret" ? sub : "limits"), pageBroken()));
    return;
  }
  const page = sub === "model" ? pageModel() : sub === "secret" ? pageSecret() : pageLimits();
  $body().replaceChildren(h("div", { class: "set" }, settingsNav(sub === "model" || sub === "secret" ? sub : "limits"), page));
  if (sub === "model" && !doctorCache) loadDoctor(false);
}

// ---- device ----
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
            c.hint ? h("div", { class: "hint" }, "→ ", c.hint) : null,
            null),
          h("span", { class: "tag" + (c.status === "ok" ? "" : " " + c.status) }, c.status === "ok" ? "Ready" : c.status === "warn" ? "Attention" : "Not ready"))),
      a === "android" && physOff ? h("div", { class: "row" }, h("div", { class: "s" }, "Phones plugged in stay off until you allow them in Limits, so only emulators are used. ", h("a", { class: "lk", href: "#/settings/limits" }, "Change it in Limits"))) : null,
    ]);
  }
  $body().replaceChildren(h("div", { class: "dev" },
    PAGES.deviceFix ? PAGES.deviceFix() : null,
    h("div", { class: "grp" }, header, content),
    h("p", { class: "muted" }, "The same checks as ", code("fastpilot doctor"), " in Terminal."),
    h("p", { class: "muted" }, "The AI model and secrets are in Settings: ", h("a", { class: "lk", href: "#/settings/model" }, "AI model"), " · ", h("a", { class: "lk", href: "#/settings/secret" }, "Secrets"), ".")));
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
  let m = /contains "([^"]+)" \(block_actions(, built-in list)?\)/.exec(n);
  if (m) return `the text contains "${asWritten("block_actions", m[1])}"${m[2] ? " (built-in list)" : ""}`;
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
    s.agent_name ? h("div", { class: "l-agent" }, h("b", {}, s.agent_name), h("span", { class: "s" }, ` · ${s.model_label || ""}`)) : null,
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
      const b = /"([^"]*)"[^"]* contains "([^"]+)" \(block_actions(, built-in list)?\)/.exec(x.note || "");
      const why = b
        ? h("span", { class: "r" }, h("b", {}, "Blocked by Limits. "), `The button text "${b[1]}" contains `, code(b[3] ? b[2] : asWritten("block_actions", b[2])), b[3] ? " (built-in list)" : "", ". The page was not pressed. ", limits(i, true))
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
      h("span", {}, "Sessions from Claude Code (MCP), the CLI or ", code("fastpilot run"), " show here live, without getting in the way of your Chrome or phone."),
      h("span", {}, "Try: ", code("fastpilot open https://example.com"))));
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
  const parts = [`${sessions.length} sessions`, `${running} running`];
  const taken = sessions.filter((s) => s.taken_over_ms).length;
  if (taken) parts.push(`${taken} taken over`);
  if (blocked) parts.push(`${blocked} blocked`);
  if (failed) parts.push(`${failed} failed`);
  return h("span", { class: "pill" + (blocked || failed ? " bad" : "") }, "● " + parts.join(" · "));
}

// D10d (v7 #s2): every agent side by side; the queue with its reasons; the parallel limit and
// today's usage per agent.
function liveExtras(data) {
  const box = document.getElementById("live-extra") || h("div", { id: "live-extra", class: "live-extra" });
  const q = data.queue || [];
  const u = data.usage || [];
  const sig = JSON.stringify([q, u]);
  if (box.dataset.sig === sig) return box;
  box.dataset.sig = sig;
  box.replaceChildren(...[
    q.length ? h("section", { class: "grp" }, h("div", { class: "gh" }, `queue · ${q.length} waiting`),
      h("ul", { class: "ag-rows q-rows" }, q.map((e) => h("li", {}, h("b", {}, e.agent || "—", h("span", { class: "s" }, ` · ${e.platform}${e.device && e.device !== "default" ? " · " + e.device : ""}${e.session !== "default" ? " · " + e.session : ""}`)),
        h("span", { class: "s" }, `waiting ${Math.max(1, Math.round((Date.now() - e.since_ms) / 60000))} min`), h("span", {}, e.reason))))) : null,
    u.length ? h("section", { class: "grp" }, h("div", { class: "gh" }, "used today, per agent"),
      h("ul", { class: "ag-rows" }, u.map((x) => h("li", {}, h("b", {}, x.agent), h("span", {}, `${x.turns} ${x.turns === 1 ? "answer or run" : "answers and runs"}`), h("span", { class: "s" }, `${x.tokens.toLocaleString("en")} tokens`))))) : null].filter(Boolean));
  return box;
}

function limitPicker(data) {
  const sel = h("select", { class: "sel", "aria-label": "Tests at once" }, Array.from({ length: 10 }, (_, i) => h("option", { value: String(i + 1), selected: data.limit === i + 1 }, `${i + 1}`)));
  sel.addEventListener("change", async () => {
    try { await api("/api/live/limit", { limit: Number(sel.value) }); toast(`Up to ${sel.value} tests at once, from the next test.`, true); } catch (e) { toast(e.message); }
  });
  return h("label", { class: "right live-limit" }, h("span", { class: "s" }, "Tests at once"), sel);
}

function renderLive(data) {
  lastLive = data;
  const sessions = data.sessions || [];
  const q = (data.queue || []).length;
  $top().replaceChildren(icon("live"), h("h1", { tabindex: "-1" }, "Live"), livePill(sessions),
    q ? h("span", { class: "pill warn" }, `${q} queued`) : null, data.limit ? limitPicker(data) : null);
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
  if ((location.hash || "#/live").startsWith("#/live")) return;
  try { lastLive = await api("/api/live"); takeBanner(lastLive.sessions); } catch (_) { /* next time */ }
}
setInterval(globalLive, 4000);

async function pollLive() {
  // An empty hash is the Live page too (the first page after the link).
  if (!(location.hash || "#/live").startsWith("#/live")) return;
  try {
    renderLive(await api("/api/live"));
  } catch (e) {
    $body().replaceChildren(h("div", { class: "err", role: "alert" }, "Cannot read the sessions: ", rich(e.message)));
  }
  liveTimer = setTimeout(pollLive, 1000);
}

// ---- Chat (C4) ----
let chatData = null, chatTimer = null, chatSig = "";
const SUGGEST = ["Test the login page in Chrome", "Write a test for the login and run it every 2 hours", "Block the Pay button in all tests"];
const chatEdits = {}; // proposal id -> what the user is changing with Edit

// D10a: whose conversation Chat shows, and where it draws (agents.js puts it in an agent's Chat tab;
// null = the Chat page itself).
let chatAgent = "default", chatHost = null;
const chatQ = () => `?agent=${encodeURIComponent(chatAgent)}`;
const chatShown = () => (location.hash || "").startsWith("#/chat") || (chatHost !== null && document.body.contains(chatHost));

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

async function chatSend(text) {
  const t = (text || "").trim();
  if (!t) return;
  try {
    chatData = await api("/api/chat", { agent: chatAgent, message: t });
    const box = document.getElementById("chat-input");
    if (box) box.value = "";
    clearToast();
  } catch (e) { toast(e.message); }
  loadChat();
}

async function chatDecide(id, apply, mode, edit, confirm) {
  try {
    const body = { id, apply, agent: chatAgent };
    if (mode) body.mode = mode;
    if (edit) body.edit = edit;
    if (confirm !== undefined) body.confirm = confirm;
    const r = await api("/api/chat/decide", body);
    delete chatEdits[id];
    chatData = r.chat;
    toast(r.message, true);
  } catch (e) { toast(e.message); await loadChat(); }
  chatSig = "";
  renderChat();
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

// A test the bot wrote (D1, v7 #s1): one card, saved only when the user clicks.
function testCard(p) {
  const ed = chatEdits[p.id];
  const head = h("h3", { class: "gh" }, icon("tests"), h("span", {}, p.schedules ? "Save test & schedule" : "Save test", " · "), h("b", { class: "t-title" }, p.title));
  let body;
  if (ed) {
    const title = h("input", { type: "text", maxlength: "80", value: ed.title, "aria-label": "Test title" });
    const steps = h("textarea", { rows: String(Math.max(3, ed.steps.length + 1)), "aria-label": "Steps, one per line" }, ed.steps.join("\n"));
    const target = h("input", { type: "text", maxlength: "500", value: ed.target, "aria-label": "Address or app" });
    const file = h("input", { type: "text", maxlength: "48", value: ed.id, class: "mono", "aria-label": "File name" });
    const read = () => ({ title: title.value, target: target.value, id: file.value, steps: steps.value.split("\n").map((x) => x.trim()).filter(Boolean) });
    // Typed text survives a refresh of the conversation while the bot answers.
    [title, steps, target, file].forEach((el) => el.addEventListener("input", () => { chatEdits[p.id] = read(); }));
    body = h("div", { class: "form t-edit" },
      h("label", { class: "fld" }, h("span", { class: "l" }, "Title"), title),
      h("label", { class: "fld" }, h("span", { class: "l" }, "Steps"), steps, h("span", { class: "s" }, "One step per line, in plain words. Secrets only as ${NAME}.")),
      h("label", { class: "fld" }, h("span", { class: "l" }, "Address or app"), target),
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
  if (p.status === "pending" && !ed) {
    foot = h("div", { class: "btns" },
      p.schedules ? h("button", { class: "btn p", onclick: () => chatDecide(p.id, true, "schedule") }, "Save & schedule") : null,
      h("button", { class: p.schedules ? "btn" : "btn p", onclick: () => chatDecide(p.id, true, "test") }, "Save test only"),
      h("button", { class: "btn", onclick: () => { chatEdits[p.id] = JSON.parse(JSON.stringify(p.draft)); chatSig = ""; renderChat(); document.querySelector(`.prop[data-pid="${p.id}"] input`)?.focus(); } }, "Edit"),
      h("button", { class: "btn quiet", onclick: () => chatDecide(p.id, false) }, "Dismiss"));
  } else if (p.status === "applied") {
    foot = [h("div", { class: "prop-state ok", tabindex: "-1" }, "✓ ", rich(p.note || "Saved")),
      h("div", { class: "btns" }, h("button", { class: "btn", onclick: () => chatRun(p.id) }, icon("play", "sm"), "Run now"),
        p.saved && p.saved.schedule ? h("a", { class: "btn", href: "#/schedules" }, "Open Schedules") : null, h("a", { class: "btn", href: "#/live" }, "Open Live"))];
  } else if (p.status === "cancelled") {
    foot = h("div", { class: "prop-state muted", tabindex: "-1" }, "Dismissed");
  }
  const note = p.status === "pending" && p.note ? h("p", { class: "s warnline" }, rich(p.note)) : null;
  const detail = p.detail ? h("details", { class: "t-detail" }, h("summary", {}, "Details"), h("p", { class: "s" }, p.detail)) : null;
  return h("section", { class: "grp prop test " + p.status, "aria-label": (p.schedules ? "Save test & schedule: " : "Save test: ") + p.title, "data-pid": String(p.id) },
    head, h("div", { class: "prop-body" }, body, note, detail, foot));
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
    foot = h("div", { class: "btns" }, h("button", { class: "btn p", onclick: () => chatDecide(p.id, false) }, "Keep as it is"), review);
  } else if (p.status === "pending") {
    foot = h("div", { class: "btns" }, h("button", { class: "btn p", onclick: () => chatDecide(p.id, true) }, "Apply"), h("button", { class: "btn", onclick: () => chatDecide(p.id, false) }, "Keep as it is"));
  } else if (p.status === "applied" && p.undo) {
    foot = h("div", { class: "btns" }, h("div", { class: "prop-state ok", tabindex: "-1" }, red ? "Allowed" : "Done"),
      h("button", { class: "btn sm", onclick: () => chatUndo(p.id) }, red ? "Block again" : "Undo"));
  } else {
    foot = h("div", { class: "prop-state muted", tabindex: "-1" }, STATE[p.status] || "");
  }
  return h("section", { class: "grp prop setting " + (red ? "loosen " : "") + p.status, "aria-label": c.title, "data-pid": String(p.id) }, head, h("div", { class: "prop-body" }, parts, foot));
}

// M1: a looser limit the bot proposed that the user did not ask for. Nothing was changed.
function pageAttempts(list) {
  return h("div", { class: "page-try", role: "status" }, icon("alert", "sm"),
    h("div", {}, h("b", {}, "A page tried to change a limit. "), "Nothing was changed. Only your own messages can ask for this.",
      h("details", {}, h("summary", {}, "See what it said"),
        list.map((a) => h("div", { class: "said" }, h("p", { class: "s" }, a.what, " (not asked by you, untrusted text):"), h("pre", {}, a.said))))));
}

// D10a: "New agent · …" (v7 #s4, right): created only on Create; Edit opens the form filled in.
function agentCard(p) {
  const rows = h("dl", { class: "prop-rows" }, p.rows.map(([k, v]) => [h("dt", {}, k), h("dd", {}, v)]));
  let foot;
  if (p.status === "pending") {
    foot = h("div", { class: "btns" },
      h("button", { class: "btn p", onclick: () => chatDecide(p.id, true) }, "Create"),
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

function proposalCard(p) {
  if (p.kind === "agent" && p.status !== "refused") return agentCard(p);
  if (p.kind === "setting" && p.setting) return settingCard(p);
  if (p.kind === "test" && p.status !== "refused") return testCard(p);
  const head = h("h3", { class: "gh" }, icon(p.kind === "schedule" ? "clock" : "shield"), p.title);
  const rows = p.rows.length ? h("dl", { class: "prop-rows" }, p.rows.map(([k, v]) => [h("dt", {}, k), h("dd", {}, v)])) : null;
  let foot;
  if (p.status === "pending") {
    foot = h("div", { class: "btns" },
      h("button", { class: "btn p", onclick: () => chatDecide(p.id, true) }, p.kind === "schedule" ? "Save the schedule" : "Apply the limits"),
      h("button", { class: "btn", onclick: () => chatDecide(p.id, false) }, "Cancel"));
  } else if (p.status === "applied") {
    foot = h("div", { class: "prop-state ok", tabindex: "-1" }, "✓ ", rich(p.note || "Saved"));
  } else if (p.status === "cancelled") {
    foot = h("div", { class: "prop-state muted", tabindex: "-1" }, "Cancelled");
  } else {
    foot = h("div", { class: "prop-state bad", tabindex: "-1" }, rich(p.note || "Cannot be used"),
      p.kind === "limits" ? [" ", h("a", { class: "lk", href: "#/settings/limits" }, "Open Settings › Limits")] : null);
  }
  const note = p.status === "pending" && p.note ? h("p", { class: "s" }, rich(p.note)) : null;
  return h("section", { class: "grp prop " + p.status, "aria-label": p.title, "data-pid": String(p.id) }, head, h("div", { class: "prop-body" }, rows, note, foot));
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

function chatMsg(m) {
  if (m.role === "user") return h("li", { class: "msg me" }, h("div", { class: "bubble" }, m.text));
  const working = !m.done;
  return h("li", { class: "msg bot" },
    h("div", { class: "who" }, icon("mark", "sm"), "fastpilot bot"),
    m.tools && m.tools.length ? h("ul", { class: "tools", "aria-label": "What the bot did" }, m.tools.map((t) => h("li", {}, icon("play", "sm"), t))) : null,
    // M3: next to a card that loosens a limit, the bot's own words are folded away.
    m.text && (m.proposals || []).some((p) => p.setting && p.setting.level === "loosen")
      ? h("details", { class: "bot-said" }, h("summary", {}, "What the bot said"), h("div", { class: "bubble" }, markdown(m.text)))
      : m.text ? botText(m.text) : null,
    working ? h("div", { class: "typing" }, h("div", { class: "spin sm", "aria-hidden": "true" }), m.tools && m.tools.length ? " Working…" : " Thinking…") : null,
    m.error ? h("div", { class: "err", role: "alert" }, rich(m.error)) : null,
    (m.proposals || []).map(proposalCard),
    m.page_attempts && m.page_attempts.length ? pageAttempts(m.page_attempts) : null);
}

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
    input.addEventListener("keydown", (e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); chatSend(input.value); } });
    root = h("div", { id: "chat-root", class: "chat" },
      h("ul", { id: "chat-log", class: "log", role: "log", "aria-live": "polite", "aria-label": "Conversation" }),
      h("div", { class: "composer" }, input, h("button", { id: "chat-send", class: "btn p", onclick: () => chatSend(input.value) }, icon("send", "sm"), "Send")),
      h("p", { class: "s muted" }, "The bot uses your Claude Code (claude -p) and fastpilot's tools. Schedules and limits only change after you click their card; through chat, limits can only get stricter."));
    if (chatHost) chatHost.append(root); else $body().replaceChildren(root);
  }
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
    log.replaceChildren(h("li", { class: "empty" }, icon("chat", "big"), h("b", {}, "Ask or tell the fastpilot bot"),
      h("span", {}, "Run tests, ask why one failed, make a schedule, or make limits stricter."),
      h("div", { class: "chips" }, SUGGEST.map((t) => h("button", { class: "chip sug", onclick: () => chatSend(t) }, t)))));
  } else {
    log.replaceChildren(...msgs.map(chatMsg));
  }
  if (atBottom || msgs.length) log.scrollTop = log.scrollHeight;
}

function pageChat() {
  // The Chat page is the first agent's conversation.
  if (chatHost || chatAgent !== "default") { chatHost = null; chatAgent = "default"; chatData = null; chatSig = ""; }
  if (!chatData) {
    $top().replaceChildren(icon("chat"), h("h1", { tabindex: "-1" }, "Chat"));
    $body().replaceChildren(h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" }), h("b", {}, "Reading the conversation…")));
  } else {
    renderChat();
  }
  loadChat();
}

// ---- Schedules (C3, D5) ----
let schedData = null, schedBusy = false, schedForm = null; // schedForm: null | { id?, ... }
const DAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const KINDS = [["day", "Fixed time"], ["hours", "Every N hours"], ["weekday", "Weekdays"], ["week", "Weekly"]];

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
  const count = l.total !== undefined && l.total !== null ? `${l.ok}/${l.total}` : (l.passed ? "passed" : "failed");
  const label = [l.passed ? "✓ " : "✗ ", count, " ", at];
  const sr = `${l.passed ? "Passed" : "Failed"} ${count} at ${at}, open the report`;
  const res = l.report
    ? h("a", { class: "lk " + (l.passed ? "ok" : "bad"), href: `/api/schedules/report?id=${encodeURIComponent(x.schedule.id)}`, target: "_blank", rel: "noopener", "aria-label": sr }, ...label)
    : h("span", { class: l.passed ? "ok" : "bad" }, ...label);
  return h("div", {}, res,
    !l.passed && l.summary ? h("div", { class: "s" }, l.summary) : null,
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
    h("td", {}, x.when_text),
    h("td", { class: "next" }, s.enabled ? (next ? when(next) : "—") : h("span", { class: "muted" }, "Off")),
    h("td", { class: "last" }, lastRun(x)),
    h("td", {}, notifyText(x)),
    h("td", {}, on),
    h("td", {}, h("div", { class: "acts" },
      h("button", { class: "btn sm", disabled: x.running, onclick: () => schedAction({ action: "run", id: s.id }, () => toast(`"${s.name}" is running now. The result shows under Last.`, true)).then(() => setTimeout(loadSchedules, 1500)) }, icon("play", "sm"), "Run now"),
      h("button", { class: "btn sm", onclick: () => { schedForm = Object.assign({}, s, { webhook_set: !!x.webhook }); pageSchedules(); document.querySelector(".sched-form input, .sched-form select")?.focus(); } }, "Edit"),
      s.notify && (s.notify.webhook || s.notify.os || s.notify.github) ? h("button", { class: "btn sm", onclick: () => schedAction({ action: "test", id: s.id }, (r) => toast("Test notification: " + r.sent.join(" · "), !r.sent.some((x) => x.includes("failed")))) }, "Test notification") : null,
      h("button", { class: "btn sm danger", onclick: () => { if (confirm(`Delete the schedule "${s.name}"? Its entry in the system scheduler is removed too.`)) schedAction({ action: "delete", id: s.id }); } }, "Delete"))));
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
  // Tests: the saved tests of this project, all of them, or another path.
  const found = (schedData && schedData.tests) || [];
  const all = schedData && schedData.tests_hint;
  const options = [];
  if (all) options.push([all, found.length === 1 ? "The 1 test in this project" : `All ${found.length} tests in this project`]);
  found.forEach((t) => options.push([t.path, t.title]));
  options.push(["", "Another folder or file…"]);
  const known = options.some(([v]) => v && v === f.tests);
  const pick = h("select", { class: "sel" }, options.map(([v, t]) => h("option", { value: v, selected: f.tests ? (known ? v === f.tests : v === "") : false }, t)));
  const path = h("input", { type: "text", value: known ? "" : (f.tests || ""), placeholder: "/full/path/to/tests or a .md file", class: "mono" });
  const pathWrap = field("Folder or file", path, "The full path of a folder with test cases (*.md), or one .md file.", "tests");
  const syncPath = () => { pathWrap.hidden = pick.value !== ""; };
  pick.addEventListener("change", () => {
    syncPath();
    if (!name.dataset.touched) name.value = pick.value ? pick.selectedOptions[0].textContent.replace(/ in this project$/, "") : "";
  });
  const name = h("input", { type: "text", maxlength: "80", value: f.name || (options.length > 1 && !f.tests ? options[0][1].replace(/ in this project$/, "") : ""), placeholder: "e.g. Checkout tests" });
  name.addEventListener("input", () => { name.dataset.touched = "1"; });
  // Kind: a radio group shown as a segmented control.
  let kind = w.every || "day";
  const seg = h("div", { class: "seg", role: "radiogroup", "aria-label": "How often" });
  const time = h("input", { type: "time", value: w.time || "08:00" });
  const hours = h("input", { type: "number", min: "1", max: "24", step: "1", value: String(w.hours || 2), class: "num", "aria-label": "Hours between runs" });
  const from = h("input", { type: "time", value: w.from || "", "aria-label": "Active from" });
  const to = h("input", { type: "time", value: w.to || "", "aria-label": "Active until" });
  const day = h("select", { class: "sel" }, DAYS.map((d, i) => h("option", { value: String(i + 1), selected: w.day === i + 1 }, d)));
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
    timeWrap.hidden = kind === "hours";
    hoursWrap.hidden = windowWrap.hidden = kind !== "hours";
    dayWrap.hidden = kind !== "week";
  };
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
  const os = h("input", { type: "checkbox", checked: !!n.os });
  const gh = h("input", { type: "checkbox", checked: !!n.github });
  const repo = h("input", { type: "text", value: n.github || "", placeholder: "owner/repo", class: "mono" });
  const whField = field("Webhook URL", whUrl, "Kept as a secret on this computer and not shown again.", "webhook");
  const repoField = field("GitHub repo", repo, h("span", {}, "Written owner/repo. Token from GITHUB_TOKEN / GH_TOKEN, or the Keychain item ", code("fastpilot-github"), "."), "repo");
  const syncNotify = () => { whField.hidden = !wh.checked; repoField.hidden = !gh.checked; };
  wh.addEventListener("change", syncNotify);
  gh.addEventListener("change", syncNotify);
  syncNotify();
  const where = (msg) => {
    const m = msg.toLowerCase();
    if (m.includes("name") && !m.includes("owner/name")) return "name";
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
  const cancel = () => { schedForm = null; clearToast(); pageSchedules(); document.querySelector(".top .btn.p")?.focus(); };
  const save = async () => {
    const whenOut = { every: kind };
    if (kind === "hours") {
      whenOut.hours = Number(hours.value);
      if (from.value || to.value) { whenOut.from = from.value || undefined; whenOut.to = to.value || undefined; }
    } else {
      whenOut.time = time.value;
      if (kind === "week") whenOut.day = Number(day.value);
    }
    const schedule = {
      id: f.id || "", name: name.value, tests: pick.value || path.value, enabled: f.enabled !== false, when: whenOut, agent: f.agent || undefined,
      notify: { webhook: wh.checked, os: os.checked, github: gh.checked ? repo.value.trim() : undefined },
    };
    const body = { action: "save", schedule };
    if (whUrl.value.trim()) body.webhook = whUrl.value.trim();
    let r;
    try { r = await api("/api/schedules", body); } catch (e) { showError(e.message); return; }
    schedForm = null;
    await loadSchedules();
    toast(editing ? "Schedule saved." : "Schedule created. It runs even when this page is closed.", true);
    // Focus on the saved schedule in the list (the form and its button are gone).
    document.querySelector(`tr[data-id="${r.id}"] .row-name`)?.focus();
  };
  syncPath();
  return h("section", { class: "grp sched-form", "aria-label": editing ? "Edit schedule" : "New schedule",
    onkeydown: (e) => { if (e.key === "Escape") { e.preventDefault(); cancel(); } } },
    h("h2", { class: "gh" }, icon("clock"), editing ? `edit schedule · ${f.name}` : "new schedule"),
    h("div", { class: "form" },
      h("div", { class: "row2" }, field("Tests", pick, null, null), field("Name", name, null, "name")),
      pathWrap,
      h("div", { class: "fld" }, h("span", { class: "l", id: "kind-l" }, "How often"), seg),
      h("div", { class: "row2" }, timeWrap, hoursWrap, windowWrap, dayWrap),
      h("fieldset", { class: "fld" }, h("legend", { class: "l" }, "Notify me when it fails (any you like)"),
        h("label", { class: "ck" }, os, " Mac notification"),
        h("label", { class: "ck" }, wh, " Slack (webhook)"), h("div", { class: "sub-fld" }, whField),
        h("label", { class: "ck" }, gh, " GitHub issue"), h("div", { class: "sub-fld" }, repoField)),
      h("div", { class: "btns" },
        h("button", { class: "btn", onclick: cancel }, "Cancel"),
        h("button", { class: "btn p", onclick: save }, editing ? "Save schedule" : "Create schedule")),
      h("p", { class: "s muted" }, "Or type it in Chat: \"run the checkout tests every 2 hours\". Escape closes this without saving.")));
}

function pageSchedules() {
  const openForm = () => { schedForm = {}; pageSchedules(); document.querySelector(".sched-form select, .sched-form input")?.focus(); };
  $top().replaceChildren(icon("clock"), h("h1", { tabindex: "-1" }, "Schedules"),
    h("button", { class: "btn sm p right", onclick: openForm }, icon("plus", "sm"), "New schedule"));
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
    : h("div", { class: "grp" }, h("div", { class: "gh" }, "schedules · empty"),
      h("div", { class: "empty" }, icon("clock", "big"), h("b", {}, "Nothing is scheduled"),
        h("span", {}, "Saved tests can run by themselves, at a fixed time or every few hours. You get a message only when something fails."),
        h("div", { class: "btns" }, h("button", { class: "btn p", onclick: openForm }, icon("plus", "sm"), "New schedule"),
          h("span", { class: "s muted" }, "or type in Chat: \"run the checkout tests every 2 hours\""))));
  $body().replaceChildren(...(schedData.error ? [h("div", { class: "err", role: "alert" }, "Could not read the schedules: " + schedData.error)] : []), ...(schedForm ? [schedPanel()] : []), table);
}

// ---- router ----
// Pages in their own files (history.js, …) register here: PAGES[name] = (sub) => …
const PAGES = {};
function render() {
  if (!S) return;
  const hash = location.hash || "#/live";
  const [, page, sub] = hash.split("/");
  if (page !== "live") stopLive();
  document.querySelectorAll(".rail a").forEach((a) => {
    const on = a.dataset.nav === page;
    a.classList.toggle("on", on);
    if (on) a.setAttribute("aria-current", "page"); else a.removeAttribute("aria-current");
  });
  if (page === "live") {
    if (!liveTimer) {
      $top().replaceChildren(icon("live"), h("h1", { tabindex: "-1" }, "Live"));
      $body().replaceChildren(h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" }), h("b", {}, "Reading sessions…")));
      pollLive();
    }
  } else if (page === "chat") {
    chatSig = "";
    pageChat();
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
  } catch (e) {
    $top().replaceChildren("fastpilot");
    $body().replaceChildren(h("div", { class: "err", role: "alert" }, "Cannot load the settings: " + e.message));
  }
});
