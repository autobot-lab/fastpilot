// fastpilot ui: Agents (D10a, v0.4; v7 #s1, #s4, #s8). A column of agents like OpenBot; each has
// Chat, Live, Schedules, Instructions, Skills, History and Agent settings. Create agent asks three
// questions (who, where, how it should work), the rest is optional; or the user describes it in
// Chat and clicks Create on the card. Uses h(), icon(), api(), toast() and the Chat functions of
// app.js. Values from the server are always set as text, never as HTML.
(() => {
"use strict";

let agData = null; // { agents: [{ agent, initials, busy, schedules }], devices, suggestions }
let agBusy = false;
let agDraft = null; // the Create agent form while it is open


async function loadAgents() {
  agBusy = true;
  try { agData = await api("/api/agents"); } catch (e) { agData = { agents: [], devices: [], suggestions: [], error: e.message }; }
  agBusy = false;
}

const byId = (id) => (agData.agents || []).find((x) => x.agent.id === id);
// Where an agent tests: chips that toggle, but the last one picked stays on. Web is picked from the
// start, and "click Web" used to switch it off, so Create then said "Pick where it tests"
// (FastTest, final report). `picked` is a Set that the caller reads on save.
const deviceChips = (picked, onChange) => {
  const hint = h("span", { class: "s dev-hint", role: "status" });
  const chips = (agData.devices || []).map((d) => {
    const b = h("button", { type: "button", class: "chip pick" + (picked.has(d.id) ? " on" : ""), "aria-pressed": String(picked.has(d.id)), "data-device": d.id }, d.name);
    b.addEventListener("click", () => {
      if (picked.has(d.id) && picked.size === 1) {
        hint.textContent = `${d.name} stays picked: an agent tests somewhere. To change it, pick another place first.`;
        return;
      }
      hint.textContent = "";
      if (picked.has(d.id)) picked.delete(d.id); else picked.add(d.id);
      b.classList.toggle("on", picked.has(d.id));
      b.setAttribute("aria-pressed", String(picked.has(d.id)));
      if (onChange) onChange();
    });
    return b;
  });
  return h("div", { class: "dev-pick" }, h("div", { class: "chips" }, chips), hint);
};
// D10b: "Provider · Model", from the server; models other than Claude are not tried with fastpilot.

// v7 #s4 step 6: the provider first, then its model. `st` is { provider, model } and is changed in place.
function modelPicker(st) {
  const box = h("div", { class: "ag-model" });
  const draw = () => {
    const prov = st.provider || "claude";
    const p = (agData.providers || []).find((x) => x.id === prov) || { name: prov, key: "" };
    const chips = h("div", { class: "chips", role: "radiogroup", "aria-label": "Provider" }, (agData.providers || []).map((x) => {
      const on = x.id === prov;
      return h("button", { type: "button", role: "radio", "aria-checked": String(on), class: "chip pick" + (on ? " on" : ""), onclick: () => { if (x.id !== prov) { st.provider = x.id; st.model = x.id === "claude" ? null : ""; draw(); } } }, x.name);
    }));
    let models;
    if (prov === "claude") {
      const cur = st.model || agData.default_model;
      models = h("div", { class: "ag-models", role: "radiogroup", "aria-label": "Model" }, (agData.claude_models || []).map((m) => {
        const on = m.id === cur;
        return h("button", { type: "button", role: "radio", "aria-checked": String(on), class: "ag-mcard" + (on ? " on" : ""), onclick: () => { st.model = m.id; draw(); } },
          h("b", {}, (on ? "✓ " : "") + m.name), h("span", { class: "s" }, m.about + (m.id === agData.default_model ? " · default" : "")));
      }));
    } else {
      const input = h("input", { type: "text", maxlength: "100", value: st.model || "", placeholder: prov === "ollama" ? "e.g. the name of a model you pulled" : "the model id at " + p.name, "aria-label": `Model at ${p.name}` });
      input.addEventListener("input", () => { st.model = input.value.trim(); });
      models = h("div", { class: "fld" }, input,
        h("span", { class: "s" }, p.key + (prov === "custom" ? " · its address is set in Settings › AI model" : "")),
        h("span", { class: "ag-untested" }, "not tested yet"), h("span", { class: "s" }, "fastpilot has not been tried with this model: run a few tests and check the results before you rely on it."));
    }
    box.replaceChildren(h("div", { class: "fld" }, h("span", { class: "l" }, "1. Provider"), chips), h("div", { class: "fld" }, h("span", { class: "l" }, "2. Model"), models),
      h("p", { class: "s muted" }, "Each provider's API key is saved once in Settings › AI model and can be used by any agent. Chat never changes the provider."));
  };
  draw();
  return box;
}

// ---- one agent ----

async function save(a, done) {
  try {
    const r = await api("/api/agents", { action: "update", agent: a });
    await loadAgents();
    toast(done || "Saved.", true);
    return r.agent;
  } catch (e) { toast(e.message); return null; }
}

function tabInstructions(x) {
  const a = x.agent;
  const input = h("input", { type: "text", maxlength: "200", placeholder: "e.g. Log in with the test account before each test", "aria-label": "New instruction" });
  const add = async (text) => {
    const t = (text || "").trim();
    if (!t) return;
    if (await save({ ...a, instructions: [...a.instructions, t] }, "Instruction added.")) page(a.id, "about");
  };
  input.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); add(input.value); } });
  const unused = (agData.suggestions || []).filter((t) => !a.instructions.includes(t));
  return h("section", { class: "grp ag-sec" },
    h("h2", { class: "gh" }, "instructions · how this agent should work"),
    h("div", { class: "ag-pad" },
      a.instructions.length
        ? h("ul", { class: "ag-rules" }, a.instructions.map((t, i) => h("li", {}, h("span", {}, t),
          h("button", { class: "rm", "aria-label": `Remove "${t}"`, onclick: async () => {
            if (await save({ ...a, instructions: a.instructions.filter((_, j) => j !== i) }, "Instruction removed.")) page(a.id, "about");
          } }, icon("close", "sm")))))
        : h("p", { class: "muted" }, "No instructions yet. The agent follows fastpilot's own rules and your limits."),
      h("div", { class: "add-row" }, input, h("button", { class: "btn", onclick: () => add(input.value) }, icon("plus", "sm"), "Add")),
      unused.length ? h("div", { class: "chips sugs" }, h("span", { class: "s" }, "Suggestions:"), unused.map((t) => h("button", { class: "chip sug", onclick: () => add(t) }, "+ ", t))) : null,
      h("p", { class: "s muted" }, "Instructions are your own rules for this agent. They never loosen a limit: blocked buttons stay blocked.")));
}

function tabSettings(x) {
  const a = x.agent;
  const name = h("input", { type: "text", maxlength: "40", value: a.name });
  const job = h("textarea", { rows: "2", maxlength: "300" }, a.job);
  const app = h("input", { type: "text", maxlength: "300", value: a.app, placeholder: "e.g. https://staging.shop.example or Shop (staging)" });
  const picked = new Set(a.devices);
  const devs = deviceChips(picked);
  const blocked = h("input", { type: "text", value: a.blocked.join(", "), placeholder: "e.g. Confirm order" });
  const fld = (l, el, hint) => h("label", { class: "fld" }, h("span", { class: "l" }, l), el, hint ? h("span", { class: "s" }, hint) : null);
  const ai = { provider: a.provider || "claude", model: a.model || null };
  const picker = modelPicker(ai);
  return h("section", { class: "grp ag-sec" }, h("h2", { class: "gh" }, "agent settings"),
    h("div", { class: "form ag-pad" },
      fld("Name", name), fld("What it does", job),
      h("div", { class: "fld" }, h("span", { class: "l" }, "Where it tests"), devs),
      fld("App or site", app),
      fld("Extra blocked words", blocked, "Only for this agent, on top of your limits; it can only be stricter. Separate with commas."),
      h("div", { class: "fld" }, h("span", { class: "l" }, "AI model"), picker),
      h("div", { class: "btns" },
        h("button", { class: "btn p", onclick: async () => {
          const out = await save({ ...a, name: name.value, job: job.value, app: app.value, devices: [...picked], blocked: blocked.value.split(",").map((w) => w.trim()).filter(Boolean),
            provider: ai.provider === "claude" ? null : ai.provider, model: ai.model || null });
          if (out) page(out.id, "about");
        } }, "Save changes"),
        a.id === "default" ? null : h("button", { class: "btn danger", onclick: async () => {
          const heir = (byId("default") || { agent: { name: "My agent" } }).agent.name;
          if (!(await UI.confirmDialog({ title: `Delete the agent "${a.name}"?`, text: `Its instructions and settings are deleted. Its schedules and conversations move to "${heir}".`, verb: "Delete agent" }))) return;
          try { await api("/api/agents", { action: "delete", id: a.id }); await loadAgents(); toast("Agent deleted.", true); location.hash = "#/agents/default/chat"; } catch (e) { toast(e.message); }
        } }, "Delete agent"))));
}

// ---- Skills (D10c, S10) ----
let libData = null;
async function library() {
  if (!libData) libData = await api("/api/skills");
  return libData;
}
// v0.7.1 FC8: the Skills tab. General QA is locked (8 skills, always on, no switch); the other
// skills are specialist switches by group, each acting at once with Undo; "Back to General QA"
// turns every specialist off. Jira skills are listed only when Jira is connected.
async function tabSkills(x, box) {
  let d;
  try { d = await api("/api/agent-skills?agent=" + encodeURIComponent(x.agent.id)); } catch (e) { box.replaceChildren(h("div", { class: "res bd", role: "alert" }, e.message)); return; }
  drawSkills(x, box, d, null);
}

function drawSkills(x, box, d, done) {
  const n = d.specialists_on;
  const change = async (body, focusId) => {
    try {
      const r = await api("/api/agent-skills", { agent: x.agent.id, ...body });
      drawSkills(x, box, r.page, r);
      const back = focusId ? box.querySelector(`[data-skill="${focusId}"]`) : box.querySelector(".sk-done");
      if (back) back.focus();
    } catch (e) { toast(e.message); }
  };
  const undo = async () => {
    try { const r = await api("/api/undo", { id: done.undo }); toast(r.message, true); } catch (e) { toast(e.message); }
    tabSkills(x, box);
  };
  const lock = () => h("span", { class: "sk-on" }, icon("lock", "sm"), "Always on");
  const row = (sk) => h("div", { class: "sk-card locked" }, h("div", { class: "sk-t" }, h("b", {}, sk.name), lock()), h("p", {}, sk.line));
  const sw = (sk) => h("div", { class: "sk-card" + (sk.on ? " on" : "") },
    h("div", { class: "sk-t" }, h("b", {}, sk.name, sk.on ? h("span", { class: "sk-added" }, "added") : null),
      h("button", { type: "button", class: "tg", role: "switch", "aria-checked": String(!!sk.on), "aria-label": sk.name, "data-skill": sk.id, onclick: () => change(sk.on ? { remove: sk.id } : { add: sk.id }, sk.id) })),
    h("p", {}, sk.line));
  box.replaceChildren(h("div", { class: "skills" },
    h("section", { class: "sk-head", "aria-label": "General QA" },
      h("div", {}, h("h2", {}, icon("lock", "sm"), n ? `General QA + ${n} specialist ${n === 1 ? "skill" : "skills"}` : "General QA · Default · Always on"),
        h("p", { class: "sub" }, n ? `8 always on, plus ${n} you added` : "8 skills · every agent has them · cannot be removed")),
      h("button", { type: "button", class: "btn", disabled: !n, onclick: () => change({ reset: true }) }, "Back to General QA")),
    done ? h("div", { class: "res sk-done", role: "status", tabindex: "-1" }, h("span", { class: "rk", "aria-hidden": "true" }, icon("check", "sm")), h("span", { class: "rt" }, done.message),
      done.undo ? h("button", { type: "button", class: "btn sm rl-undo", onclick: undo }, "Undo") : null) : null,
    h("div", { class: "sk-grid" }, d.general.map(row)),
    h("h2", { class: "sk-h2" }, "Specialist skills"), h("p", { class: "sub" }, "switch on to add, switch off to remove"),
    d.groups.map((g) => h("section", { class: "sk-group", "aria-label": g.name }, h("h3", {}, g.name), h("p", { class: "sub" }, g.about), h("div", { class: "sk-grid" }, g.skills.map(sw)))),
    d.jira ? null : h("p", { class: "sub" }, "Jira skills appear after you connect Jira.")));
}

// FC4: an agent's own pages inside the frame (shell.js draws its name and tabs): Skills, and About
// this agent (its instructions and settings). Chat is app.js's; the other tabs are their pages.
function page(id, tab) {
  const x = byId(id);
  if (!x) { location.replace("#/agents/default/chat"); return; }
  const a = x.agent;
  const elsewhere = { live: "#/live", schedules: "#/schedules", history: "#/history", memory: "#/memory", map: "#/map" };
  if (elsewhere[tab]) { location.replace(elsewhere[tab]); return; }
  const box = h("div", { class: "ag-main col2" });
  const title = tab === "skills" ? "Skills" : "About this agent";
  $top().replaceChildren(h("h1", { tabindex: "-1" }, title));
  $body().replaceChildren(box);
  if (tab === "skills") tabSkills(x, box);
  else box.replaceChildren(tabInstructions(x), tabSettings(x));
}

// ---- Create agent (v7 #s4) ----

// S4: one click makes an agent with a focus; where it tests, the app and its skills come from the
// current app. Describing one yourself (the full form, filled in from the app) is under More options.
async function createFocus(id, btn) {
  btn.disabled = true;
  try {
    const r = await api("/api/agents", { action: "create_focus", focus: id });
    agDraft = null;
    await loadAgents();
    toast(`"${r.agent.name}" is ready.`, true);
    location.hash = `#/agents/${r.agent.id}/chat`;
  } catch (e) { btn.disabled = false; toast(e.message); }
}

function pageNew() {
  chatHost = null;
  const nd = agData.new_defaults || {};
  const opened = !!agDraft; // Edit on a Chat card, or a form the user already started
  const d = agDraft || (agDraft = { name: "", job: "", devices: (nd.devices && nd.devices.length ? nd.devices : ["web"]).slice(), app: nd.app || "", instructions: [], blocked: "", ai: { provider: "claude", model: null } });
  d.ai = d.ai || { provider: "claude", model: null };
  const errs = h("p", { class: "fld-err", role: "alert", hidden: true });
  const name = h("input", { type: "text", maxlength: "40", value: d.name, placeholder: "e.g. Mobile Payments", "aria-label": "Name" });
  name.addEventListener("input", () => { d.name = name.value; });
  const job = h("textarea", { rows: "2", maxlength: "300", placeholder: "e.g. Keep cart and payment working in the Android and iOS app on every release", "aria-label": "What it does" }, d.job);
  job.addEventListener("input", () => { d.job = job.value; });
  if (!d.devices.length) d.devices = ["web"];
  const pickedNew = new Set(d.devices);
  const devs = deviceChips(pickedNew, () => { d.devices = (agData.devices || []).map((v) => v.id).filter((id) => pickedNew.has(id)); });
  const app = h("input", { type: "text", maxlength: "300", value: d.app, placeholder: "App or site, e.g. https://staging.shop.example", "aria-label": "App or site" });
  app.addEventListener("input", () => { d.app = app.value; });
  const rules = h("ul", { class: "ag-rules" });
  const sugs = h("div", { class: "chips sugs" });
  const drawRules = () => {
    rules.replaceChildren(...d.instructions.map((t, i) => h("li", {}, h("span", {}, t), h("button", { class: "rm", "aria-label": `Remove "${t}"`, onclick: () => { d.instructions.splice(i, 1); drawRules(); } }, icon("close", "sm")))));
    sugs.replaceChildren(h("span", { class: "s" }, "Suggestions:"), ...(agData.suggestions || []).filter((t) => !d.instructions.includes(t)).map((t) => h("button", { type: "button", class: "chip sug", onclick: () => { d.instructions.push(t); drawRules(); } }, "+ ", t)));
  };
  drawRules();
  const rule = h("input", { type: "text", maxlength: "200", placeholder: "Your own rule, e.g. Never press Pay; stop at the order summary", "aria-label": "New instruction" });
  const addRule = () => { const t = rule.value.trim(); if (t) { d.instructions.push(t); rule.value = ""; drawRules(); } };
  rule.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); addRule(); } });
  const blocked = h("input", { type: "text", value: d.blocked, placeholder: "e.g. Confirm order", "aria-label": "Extra blocked words" });
  blocked.addEventListener("input", () => { d.blocked = blocked.value; });
  // D10c: 3–4 suggested skills, ticked, each with its reason; the rest by pack, off.
  const skillStep = h("div", { class: "sk-step" }, h("p", { class: "s muted" }, "Loading skills…"));
  const drawSkills = async () => {
    let lib, sug;
    try {
      lib = await library();
      sug = (await api("/api/agents", { action: "suggest", agent: { job: d.job, app: d.app, devices: d.devices } })).suggested;
    } catch (e) { skillStep.replaceChildren(h("p", { class: "err" }, e.message)); return; }
    if (!d.skillsTouched) d.skills = [lib.chooser, ...sug.map((x) => x.id)];
    const why = Object.fromEntries(sug.map((x) => [x.id, x.why]));
    const chip = (sk) => {
      const onNow = d.skills.includes(sk.id);
      return h("button", { type: "button", class: "chip pick" + (onNow ? " on" : ""), "aria-pressed": String(onNow), onclick: () => { d.skillsTouched = true; d.skills = onNow ? d.skills.filter((x) => x !== sk.id) : [...d.skills, sk.id]; drawSkills(); } },
        (onNow ? "✓ " : "") + sk.name, why[sk.id] ? h("span", { class: "why" }, ` · suggested: ${why[sk.id]}`) : null);
    };
    skillStep.replaceChildren(
      h("div", { class: "chips" }, lib.skills.filter((sk) => why[sk.id]).map(chip)),
      h("details", { class: "sk-more" }, h("summary", {}, "More skills, by pack"),
        lib.packs.map((p) => h("div", { class: "fld" }, h("span", { class: "l" }, `${p.name} · ${p.about}`), h("div", { class: "chips" }, lib.skills.filter((sk) => sk.pack === p.id && !why[sk.id]).map(chip))))),
      h("p", { class: "s muted" }, "Skills are ready-made know-how (how to test a form, how to run a regression). Instructions are your own rules."));
  };
  drawSkills();
  job.addEventListener("change", () => { if (!d.skillsTouched) drawSkills(); });
  const create = async () => {
    errs.hidden = true;
    try {
      const r = await api("/api/agents", { action: "create", agent: { id: "", name: d.name, job: d.job, devices: d.devices, app: d.app, instructions: d.instructions, blocked: d.blocked.split(",").map((w) => w.trim()).filter(Boolean),
        provider: d.ai.provider === "claude" ? null : d.ai.provider, model: d.ai.model || null, skills: d.skills || [] } });
      agDraft = null;
      await loadAgents();
      toast(`Created "${r.agent.name}".`, true);
      location.hash = `#/agents/${r.agent.id}/chat`;
    } catch (e) { errs.textContent = e.message; errs.hidden = false; errs.focus(); }
  };
  const forApp = nd.app_name ? `New agent for ${nd.app_name}` : "New agent";
  $top().replaceChildren(icon("plus"), h("h1", { tabindex: "-1" }, forApp),
    h("span", { class: "right btns tight" }, h("a", { class: "btn", href: "#/agents" }, "Cancel")));
  const sec = (n, title, opt, ...kids) => h("section", { class: "grp ag-sec" }, h("h2", { class: "gh" }, `${n} · ${title}`, opt ? h("span", { class: "opt" }, " · optional") : null), h("div", { class: "ag-pad" }, ...kids));
  const toChat = h("button", { class: "btn", onclick: () => {
    location.hash = "#/agents/default/chat";
    setTimeout(() => { const box = document.getElementById("chat-input"); if (box) { box.value = "Create an agent for checkout that never presses Pay"; box.focus(); } }, 300);
  } }, icon("chat", "sm"), "Describe it in Chat");
  const focuses = h("div", { class: "prop-body ag-focus" }, (agData.focuses || []).map((f) => {
    const b = h("button", { class: "say-row", "data-focus": f.id }, h("span", {}, f.label), h("span", { class: "arrow", "aria-hidden": "true" }, "→"));
    b.addEventListener("click", () => createFocus(f.id, b));
    return b;
  }));
  $body().replaceChildren(h("div", { class: "ag-new" },
    h("section", { class: "grp ag-sec ag-quick" },
      h("div", { class: "ag-pad" }, h("p", {}, "Most apps need only the tester fastpilot already made. Add another with a focus:"), focuses,
        h("p", { class: "s muted" }, "Where it tests, the app and its skills come from ", nd.app_name ? `${nd.app_name}` : "this app", "; change them later in Agent settings."))),
    UI.section("Describe it yourself",
      h("div", { class: "ag-form" }, errs,
        sec(1, "who", false, h("label", { class: "fld" }, h("span", { class: "l" }, "Name"), name), h("label", { class: "fld" }, h("span", { class: "l" }, "What it does"), job)),
        sec(2, "where", false, devs, app),
        sec(3, "instructions (how this agent should work)", true, rules, h("div", { class: "add-row" }, rule, h("button", { type: "button", class: "btn", onclick: addRule }, icon("plus", "sm"), "Add")), sugs),
        sec(4, "skills", true, skillStep),
        sec(5, "this agent's AI model", true, modelPicker(d.ai)),
        sec(6, "this agent's limits", true, h("p", { class: "s" }, "It follows your global limits (pay, checkout and delete account are blocked). An agent can only be stricter."),
          h("label", { class: "fld" }, h("span", { class: "l" }, "Extra blocked words for this agent"), blocked)),
        h("div", { class: "btns" }, h("button", { class: "btn p", onclick: create }, "Create agent"))),
      h("section", { class: "grp ag-side" }, h("h2", { class: "gh" }, "or in chat"),
        h("div", { class: "ag-pad" }, h("p", {}, "Tell fastpilot in one sentence what the agent should do. It prepares a card; nothing is created until you click Create."), toChat)))));
  const first = opened ? name : focuses.querySelector("button");
  if (first) first.focus();
}

async function route() {
  // Read again on every visit: an agent may have been made from a Chat card meanwhile.
  if (!agBusy) await loadAgents();
  if (!agData) return;
  const [, , id, tab] = (location.hash || "").split("/");
  if (!id) { location.hash = "#/agents/default/chat"; return; }
  if (id === "create") pageNew(); else page(id, tab);
}

PAGES.agents = () => { route(); };
// Edit on a "New agent" card in Chat: the form, filled in with what the bot proposed.
window.editAgentDraft = (d) => {
  agDraft = { name: d.name || "", job: d.job || "", devices: d.devices || ["web"], app: d.app || "", instructions: d.instructions || [], blocked: (d.blocked || []).join(", ") };
  location.hash = "#/agents/create";
};
})();
