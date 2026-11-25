// fastpilot ui: E8 + E6 (v0.7.2). A model per agent (the Model tab) and the providers with their
// model lists (Settings › AI model: Default models and Providers). Design g19a–g19l; every sentence
// that comes from the API (message, note, warn, fallback, line, error) is shown as written.
// Exposed as one global, Models.
(() => {
"use strict";

const ROLES = [
  ["replay", "Replay", "Repeats steps that are already recorded. Fast and cheap."],
  ["explore", "Explore and repair", "Used for a new test, or when a replay fails."],
];
const PILL = { connected: ["Connected", "ok"], not_running: ["Not running", "bad"], key_not_accepted: ["Key not accepted", "bad"], not_answering: ["Not answering", "bad"] };

let provs = null; // GET /api/providers

/// The API's own sentence for a refusal (422 `{error, code}`), else the text as it came.
function said(e) {
  try { return JSON.parse(e.message).error || e.message; } catch (_) { return e.message; }
}
async function call(path, body) {
  try { return await api(path, body); } catch (e) { throw new Error(said(e)); }
}
async function providers(force) {
  if (!provs || force) provs = await call("/api/providers");
  return provs;
}

/// A result line with Undo, as a toast (the API's own message; Undo through /api/undo).
function done(r, after) {
  document.querySelectorAll(".toast").forEach((t) => t.remove());
  const lines = [r.note, ...(r.lines || [])].filter((l, i, a) => l && a.indexOf(l) === i);
  const t = h("div", { class: "toast ok", role: "status" }, h("span", {}, r.message),
    lines.length ? h("small", { class: "toast-sub" }, lines.join(" ")) : null,
    r.undo ? h("button", { type: "button", class: "btn sm", onclick: async () => {
      t.remove();
      try { const u = await call("/api/undo", { id: r.undo }); toast(u.message, true); } catch (e) { toast(e.message); }
      if (after) after();
    } }, "Undo") : null);
  document.body.append(t);
  setTimeout(() => t.remove(), 12000);
}

// ---- the model picker (g19b) -----------------------------------------------------------------

/// The button for one role: the model's name in bold, "provider · note" under it; a provider that
/// does not answer (or a stopped Ollama) gets the red outline. Opens the list on click.
function pickerButton(role, title, x, onPick) {
  const e = x.effective || {};
  const sub = x.is_default ? `Default · ${x.provider_name}` : [x.provider_name, x.host && x.host !== x.provider_name ? x.host : null].filter(Boolean).join(" · ");
  const bad = x.state === "not_answering" || x.state === "not_running";
  const btn = h("button", { type: "button", class: "mpick" + (bad ? " bad" : ""), "aria-haspopup": "listbox", "aria-expanded": "false", "aria-label": `${title}: ${e.name || e.model || "Default"}. Choose a model` },
    h("b", {}, e.name || e.model || "Default"), h("small", {}, bad ? `${x.host || x.provider_name} · ${PILL[x.state][0]}` : sub), icon("chev", "sm"));
  btn.addEventListener("click", () => openList(btn, role, x, onPick));
  return btn;
}

function openList(btn, role, x, onPick) {
  document.getElementById("mlist")?.remove();
  btn.setAttribute("aria-expanded", "true");
  const close = (back) => { box.remove(); btn.setAttribute("aria-expanded", "false"); document.removeEventListener("mousedown", outside, true); if (back) btn.focus(); };
  const outside = (ev) => { if (!box.contains(ev.target) && ev.target !== btn) close(false); };
  const search = h("input", { type: "text", class: "msearch", placeholder: "Search models", "aria-label": "Search models", autocomplete: "off", spellcheck: "false" });
  const list = h("div", { class: "mopts", role: "listbox", "aria-label": "Models" });
  const box = h("div", { id: "mlist", class: "mlist" }, search, list,
    h("a", { class: "lk s mfoot", href: "#/settings/model", onclick: () => close(false) }, "Add a provider in Settings ›"));
  let more = {}; // provider id -> show all
  const chosen = x.choice ? `${x.choice.provider}:${x.choice.model}` : "default";
  const pick = async (value) => { close(true); await onPick(role, value); };
  const opt = (value, label, sub, mono) => h("div", { class: "mopt" + (value === chosen ? " on" : ""), role: "option", tabindex: "-1", "aria-selected": String(value === chosen), "data-v": value,
    onclick: () => pick(value) }, h("span", { class: mono ? "mono" : "" }, label), sub ? h("small", {}, sub) : null, value === chosen ? icon("check", "sm") : null);
  const draw = () => {
    const q = search.value.trim().toLowerCase();
    const rows = [];
    const d = (provs && provs.defaults && provs.defaults[role]) || {};
    if (!q || "default".includes(q)) rows.push(opt("default", "Default", `${d.name || d.model || ""} (from Settings)`));
    for (const p of (provs && provs.providers) || []) {
      const ms = (p.models || []).filter((m) => !q || m.id.toLowerCase().includes(q) || (m.name || "").toLowerCase().includes(q));
      const head = p.id === "claude" ? "Claude Code · uses your login" : p.local && p.id === "ollama" ? "Ollama · this Mac" : p.name;
      const refresh = p.id === "claude" ? null : h("button", { type: "button", class: "btn sm quiet", "aria-label": `Refresh models from ${p.name}`, onclick: async (ev) => {
        ev.stopPropagation();
        try { const r = await call("/api/providers", { action: "refresh", provider: p.id }); toast(r.message, true); await providers(true); draw(); } catch (e) { toast(e.message); }
      } }, "Refresh");
      const group = [h("div", { class: "mgroup", role: "presentation" }, h("span", {}, head), refresh)];
      if (p.models_error) group.push(h("p", { class: "s bad mnote" }, p.models_error));
      else if (p.id === "ollama" && !(p.models || []).length && p.state === "connected") group.push(h("p", { class: "s muted mnote" }, "Ollama is running but has no models yet. Download one in Ollama, then press Refresh."));
      const shown = q || more[p.id] ? ms : ms.slice(0, 3);
      for (const m of shown) group.push(opt(`${p.id}:${m.id}`, m.name || m.id, p.local ? "Stays on this Mac" : null, p.id !== "claude"));
      if (!q && !more[p.id] && ms.length > 3) group.push(h("button", { type: "button", class: "btn sm quiet mmore", onclick: (ev) => { ev.stopPropagation(); more[p.id] = true; draw(); } }, `Show ${ms.length - 3} more`));
      if (!q || ms.length) rows.push(...group);
    }
    if (!rows.length) rows.push(h("p", { class: "s muted mnote" }, "No model matches."));
    list.replaceChildren(...rows);
  };
  const options = () => [...list.querySelectorAll('[role="option"]')];
  box.addEventListener("keydown", (ev) => {
    const all = options();
    const i = all.indexOf(document.activeElement);
    if (ev.key === "Escape") { ev.preventDefault(); close(true); }
    else if (ev.key === "ArrowDown") { ev.preventDefault(); (all[i + 1] || all[0])?.focus(); }
    else if (ev.key === "ArrowUp") { ev.preventDefault(); (i <= 0 ? search : all[i - 1]).focus(); }
    else if (ev.key === "Enter" && i >= 0) { ev.preventDefault(); pick(all[i].dataset.v); }
  });
  search.addEventListener("input", draw);
  draw();
  btn.after(box);
  document.addEventListener("mousedown", outside, true);
  search.focus();
}

// ---- one role row (Model tab and Default models) ----------------------------------------------

function roleRow(role, title, about, x, onPick, useDefault, recheck) {
  const bad = x.state === "not_answering" || x.state === "not_running";
  const who = x.host || x.provider_name;
  return h("section", { class: "srow mrow", "aria-label": title },
    h("div", { class: "mrow-l" }, h("b", {}, title), h("small", {}, about)),
    h("div", { class: "mrow-r" },
      h("div", { class: "btns tight" }, pickerButton(role, title, x, onPick),
        useDefault && x.choice ? h("button", { type: "button", class: "lk s", onclick: () => onPick(role, "default") }, "Use default") : null),
      x.fallback ? h("p", { class: "s muted" }, x.fallback) : null,
      bad ? h("div", { class: "mnote bad", role: "alert" },
        h("p", {}, x.state === "not_running" ? "Not running. Start Ollama on this Mac, then check again."
          : `${who} did not answer. Tests that need ${title} will stop with this message until it does. Nothing was switched for you.`),
        h("div", { class: "btns tight" }, h("button", { type: "button", class: "btn sm", onclick: recheck }, "Check again"),
          useDefault && x.choice ? h("button", { type: "button", class: "btn sm", onclick: () => onPick(role, "default") }, "Use default") : null)) : null,
      x.warn ? h("div", { class: "mnote warn" }, h("b", {}, "Not tried with fastpilot yet. "), x.warn, x.note ? " " + x.note : "")
        : x.note ? h("p", { class: "s muted" }, x.note) : null));
}

// ---- the Model tab (g19a–g19f) ---------------------------------------------------------------

async function agentTab(id, box) {
  box.replaceChildren(h("p", { class: "s muted", role: "status" }, "Reading models…"));
  let d;
  try { [d] = await Promise.all([call("/api/agent-model?agent=" + encodeURIComponent(id)), providers(true)]); } catch (e) { box.replaceChildren(UI.notice("error", e.message)); return; }
  const again = () => agentTab(id, box);
  const onPick = async (role, value) => {
    try { done(await call("/api/agent-model", { agent: id, [role]: value }), again); } catch (e) { toast(e.message); }
    await again();
  };
  const recheck = (x) => async () => {
    try { const r = await call("/api/providers", { action: "refresh", provider: (x.effective || {}).provider }); toast(r.message, true); } catch (e) { toast(e.message); }
    await again();
  };
  box.replaceChildren(h("div", { class: "model-tab" },
    h("h2", {}, "Model"),
    h("p", { class: "sub" }, "Which AI runs this agent’s tests. Each agent can use its own."),
    ROLES.map(([role, title, about]) => roleRow(role, title, about, d[role], onPick, true, recheck(d[role]))),
    h("p", { class: "s muted" }, "New agents start with the defaults in ", h("a", { class: "lk", href: "#/settings/model" }, "Settings › AI model"), ".")));
}

// ---- Settings › AI model (g19g–g19l) ----------------------------------------------------------

let adding = null; // the addable row being connected (g19i)
let editing = null; // provider id whose Edit form is open

function settingsPage(host) {
  const box = h("div", { class: "model-settings" }, h("p", { class: "s muted", role: "status" }, "Reading providers…"));
  host.replaceChildren(box);
  fill(box);
  return host;
}

async function fill(box) {
  let d;
  try { [d] = await Promise.all([call("/api/agent-model"), providers(true)]); } catch (e) { box.replaceChildren(UI.notice("error", e.message)); return; }
  const again = () => fill(box);
  const onPick = async (role, value) => {
    try { done(await call("/api/agent-model", { default: true, [role]: value }), again); } catch (e) { toast(e.message); }
    await again();
  };
  const recheck = (x) => async () => {
    try { const r = await call("/api/providers", { action: "refresh", provider: (x.effective || {}).provider }); toast(r.message, true); } catch (e) { toast(e.message); }
    await again();
  };
  const addBtn = h("button", { type: "button", class: "btn", "aria-haspopup": "menu", "aria-expanded": "false" }, "+ Add a provider");
  addBtn.addEventListener("click", () => addMenu(addBtn, again));
  box.replaceChildren(...UI.kids(
    h("h2", {}, "AI model"),
    h("p", { class: "sub" }, "Defaults for every agent. Each agent can choose its own on its Model tab."),
    h("h3", { class: "more-h" }, "Default models"),
    ROLES.map(([role, title, about]) => roleRow(role, title, about, d[role], onPick, false, recheck(d[role]))),
    h("div", { class: "mhead" }, h("h3", { class: "more-h" }, "Providers"), addBtn),
    adding ? connectCard(adding, again) : null,
    (provs.providers || []).map((p) => providerCard(p, again)),
    provs.keys_kept_in ? h("p", { class: "s muted" }, `API keys are kept in ${provs.keys_kept_in}, never shown again, and sent only to their own provider.`) : null));
}

function addMenu(btn, again) {
  document.getElementById("addmenu")?.remove();
  btn.setAttribute("aria-expanded", "true");
  const close = (back) => { menu.remove(); btn.setAttribute("aria-expanded", "false"); if (back) btn.focus(); };
  const items = (provs.addable || []).map((a) => h("button", { type: "button", role: "menuitem", class: "amenu-i", disabled: a.added, onclick: () => { close(false); adding = a; again(); } },
    h("b", {}, a.name), h("small", {}, a.added ? `${a.line} · already added` : a.line)));
  const menu = h("div", { id: "addmenu", class: "amenu", role: "menu", "aria-label": "Add a provider" }, items);
  menu.addEventListener("keydown", (ev) => {
    const all = [...menu.querySelectorAll("button:not([disabled])")];
    const i = all.indexOf(document.activeElement);
    if (ev.key === "Escape") { ev.preventDefault(); close(true); }
    else if (ev.key === "ArrowDown") { ev.preventDefault(); (all[i + 1] || all[0]).focus(); }
    else if (ev.key === "ArrowUp") { ev.preventDefault(); (all[i - 1] || all[all.length - 1]).focus(); }
  });
  btn.after(menu);
  menu.querySelector("button:not([disabled])")?.focus();
}

/// g19i: connecting a provider: one masked key field (and the address for Other), the provider's own
/// line from the API (where page text goes), Connect and Cancel. The key is sent once, never kept here.
function connectCard(a, again) {
  const key = a.needs_key || a.id === "other" ? h("input", { type: "password", class: "mono", autocomplete: "off", spellcheck: "false", "aria-label": `${a.name} API key`, placeholder: a.needs_key ? "Paste the API key" : "API key (not needed on this computer)" }) : null;
  const url = a.needs_address ? h("input", { type: "text", class: "mono", spellcheck: "false", "aria-label": "Address", placeholder: "https://llm.example.com/v1" }) : null;
  const out = h("p", { class: "s", role: "status" });
  const go = h("button", { type: "button", class: "btn p" }, "Connect");
  go.addEventListener("click", async () => {
    go.disabled = true;
    out.textContent = "Connecting…";
    try {
      const r = await call("/api/providers", { action: "add", provider: a.id, ...(key && key.value ? { key: key.value } : {}), ...(url ? { base_url: url.value } : {}) });
      if (key) key.value = "";
      adding = null;
      done(r, again);
      await again();
    } catch (e) { out.textContent = e.message; out.className = "s bad"; go.disabled = false; }
  });
  [key, url].filter(Boolean).forEach((el) => el.addEventListener("keydown", (ev) => { if (ev.key === "Enter") go.click(); }));
  queueMicrotask(() => (url || key)?.focus());
  return h("section", { class: "pcard pconnect", "aria-label": `Connect ${a.name}` },
    h("div", { class: "pcard-h" }, h("span", { class: "plogo", "aria-hidden": "true" }, a.name.charAt(0)), h("b", {}, a.name)),
    url ? h("label", { class: "fld" }, h("span", { class: "l" }, "Address"), url) : null,
    key ? h("label", { class: "fld" }, h("span", { class: "l" }, "API key"), key) : null,
    h("p", { class: "s muted" }, a.line),
    h("div", { class: "btns" }, go, h("button", { type: "button", class: "btn", onclick: () => { adding = null; again(); } }, "Cancel")), out);
}

/// One provider: letter, name, its line, the connection pill, Refresh and Edit (Edit opens the
/// address for Other/Ollama, a new key and a model id); Remove where it can be removed.
function providerCard(p, again) {
  const [pill, tone] = PILL[p.state] || ["", ""];
  const refresh = async () => {
    try { const r = await call("/api/providers", { action: "refresh", provider: p.id }); toast(r.message, true); } catch (e) { toast(e.message); }
    await again();
  };
  const remove = async () => {
    if (!(await UI.confirmDialog({ title: `Remove ${p.name}?`, text: "Its API key is deleted from this computer. This cannot be undone. Agents that use it go back to the default.", verb: "Remove provider" }))) return;
    try { const r = await call("/api/providers", { action: "remove", provider: p.id }); done(r, again); } catch (e) { toast(e.message); }
    await again();
  };
  const isEditing = editing === p.id;
  return h("section", { class: "pcard", "aria-label": p.name },
    h("div", { class: "pcard-h" }, h("span", { class: "plogo", "aria-hidden": "true" }, p.name.charAt(0).toUpperCase()),
      h("div", { class: "pcard-t" }, h("b", {}, p.name), h("small", {}, p.line)),
      p.badge ? h("span", { class: "tag" }, p.badge) : pill ? h("span", { class: "tag " + tone }, pill) : null,
      p.id === "claude" ? null : h("span", { class: "btns tight" },
        h("button", { type: "button", class: "btn sm", "aria-label": `Refresh ${p.name}`, onclick: refresh }, "Refresh"),
        h("button", { type: "button", class: "btn sm", "aria-expanded": String(isEditing), "aria-label": `Edit ${p.name}`, onclick: () => { editing = isEditing ? null : p.id; again(); } }, "Edit"),
        p.removable ? h("button", { type: "button", class: "btn sm d", "aria-label": `Remove ${p.name}`, onclick: remove }, "Remove") : null)),
    p.message ? h("p", { class: "s " + (tone === "bad" ? "bad" : "muted") }, p.message) : null,
    p.models_error ? h("p", { class: "s bad" }, p.models_error) : null,
    isEditing ? editForm(p, again) : null);
}

function editForm(p, again) {
  const url = ["other", "ollama"].includes(p.id) ? h("input", { type: "text", class: "mono", spellcheck: "false", value: p.base_url || "", "aria-label": "Address" }) : null;
  const key = p.id === "ollama" ? null : h("input", { type: "password", class: "mono", autocomplete: "off", "aria-label": `${p.name} API key`, placeholder: p.has_key ? "Saved · type a new key to replace it" : "Paste the API key" });
  const model = h("input", { type: "text", class: "mono", spellcheck: "false", "aria-label": "Model id", placeholder: "Add a model id, for example my/model" });
  const out = h("p", { class: "s", role: "status" });
  const save = h("button", { type: "button", class: "btn p" }, "Save changes");
  save.addEventListener("click", async () => {
    save.disabled = true;
    const body = { action: "edit", provider: p.id };
    if (url && url.value.trim() && url.value.trim() !== (p.base_url || "")) body.base_url = url.value.trim();
    if (key && key.value) body.key = key.value;
    if (model.value.trim()) body.model_id = model.value.trim();
    try { const r = await call("/api/providers", body); if (key) key.value = ""; editing = null; done(r, again); await again(); }
    catch (e) { out.textContent = e.message; out.className = "s bad"; save.disabled = false; }
  });
  return h("div", { class: "pedit" },
    url ? h("label", { class: "fld" }, h("span", { class: "l" }, "Address"), url) : null,
    key ? h("label", { class: "fld" }, h("span", { class: "l" }, "API key"), key) : null,
    h("label", { class: "fld" }, h("span", { class: "l" }, "Model id"), model, h("span", { class: "s" }, "For a model the list does not show.")),
    h("div", { class: "btns" }, save, h("button", { type: "button", class: "btn", onclick: () => { editing = null; again(); } }, "Cancel")), out);
}

window.Models = { agentTab, settingsPage };
})();
