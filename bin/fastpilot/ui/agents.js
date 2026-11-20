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

const TABS = [["chat", "Chat"], ["live", "Live"], ["schedules", "Schedules"], ["instructions", "Instructions"], ["skills", "Skills"], ["history", "History"], ["settings", "Agent settings"]];
const COLORS = 7; // .ag-c0 … .ag-c6 in app.css (the page's policy allows no inline styles)

async function loadAgents() {
  agBusy = true;
  try { agData = await api("/api/agents"); } catch (e) { agData = { agents: [], devices: [], suggestions: [], error: e.message }; }
  agBusy = false;
}

const byId = (id) => (agData.agents || []).find((x) => x.agent.id === id);
function color(id) {
  let n = 0;
  for (const c of id) n = (n * 31 + c.charCodeAt(0)) >>> 0;
  return n % COLORS;
}
const avatar = (x, cls) => h("span", { class: `ag-av ag-c${color(x.agent.id)} ${cls || ""}`, "aria-hidden": "true" }, x.initials);
const deviceName = (d) => ((agData.devices || []).find((x) => x.id === d) || { name: d }).name;
// D10b fills in the provider and model; until then every agent uses the global AI settings.
const modelLabel = (a) => (a.provider || a.model ? `${a.provider || "Claude Code"} · ${a.model || "default"}` : "Claude Code · default");

function summary(x) {
  const a = x.agent;
  const where = a.devices.map(deviceName).join(" + ");
  return x.schedules.length ? `${x.schedules[0]} · ${where}` : where;
}

function list(current) {
  const items = (agData.agents || []).map((x) => h("li", {},
    h("a", { class: "ag-item" + (x.agent.id === current ? " on" : ""), href: `#/agents/${x.agent.id}/chat`, "aria-current": x.agent.id === current ? "page" : null },
      avatar(x),
      h("span", { class: "ag-txt" }, h("b", {}, x.agent.name), h("span", { class: "s" }, summary(x)), h("span", { class: "s" }, modelLabel(x.agent))),
      h("span", { class: "ag-st " + (x.busy ? "run" : "ready") }, x.busy ? "running" : "ready"))));
  return h("nav", { class: "ag-col", "aria-label": "Agents" },
    h("div", { class: "ag-col-head" }, h("h2", {}, "Agents"), h("a", { class: "btn sm p", href: "#/agents/new" }, icon("plus", "sm"), "New agent")),
    h("ul", { class: "ag-list" }, items));
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
    if (await save({ ...a, instructions: [...a.instructions, t] }, "Instruction added.")) page(a.id, "instructions");
  };
  input.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); add(input.value); } });
  const unused = (agData.suggestions || []).filter((t) => !a.instructions.includes(t));
  return h("section", { class: "grp ag-sec" },
    h("h2", { class: "gh" }, "instructions · how this agent should work"),
    h("div", { class: "ag-pad" },
      a.instructions.length
        ? h("ul", { class: "ag-rules" }, a.instructions.map((t, i) => h("li", {}, h("span", {}, t),
          h("button", { class: "rm", "aria-label": `Remove "${t}"`, onclick: async () => {
            if (await save({ ...a, instructions: a.instructions.filter((_, j) => j !== i) }, "Instruction removed.")) page(a.id, "instructions");
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
  const devs = h("div", { class: "chips" }, (agData.devices || []).map((d) => {
    const b = h("button", { type: "button", class: "chip pick" + (picked.has(d.id) ? " on" : ""), "aria-pressed": String(picked.has(d.id)) }, d.name);
    b.addEventListener("click", () => { picked.has(d.id) ? picked.delete(d.id) : picked.add(d.id); b.classList.toggle("on"); b.setAttribute("aria-pressed", String(picked.has(d.id))); });
    return b;
  }));
  const blocked = h("input", { type: "text", value: a.blocked.join(", "), placeholder: "e.g. Confirm order" });
  const fld = (l, el, hint) => h("label", { class: "fld" }, h("span", { class: "l" }, l), el, hint ? h("span", { class: "s" }, hint) : null);
  return h("section", { class: "grp ag-sec" }, h("h2", { class: "gh" }, "agent settings"),
    h("div", { class: "form ag-pad" },
      fld("Name", name), fld("What it does", job),
      h("div", { class: "fld" }, h("span", { class: "l" }, "Where it tests"), devs),
      fld("App or site", app),
      fld("Extra blocked words", blocked, "Only for this agent, on top of your limits; it can only be stricter. Separate with commas."),
      h("div", { class: "fld" }, h("span", { class: "l" }, "AI model"), h("span", {}, modelLabel(a)), h("span", { class: "s" }, "Picking a provider and model per agent comes next.")),
      h("div", { class: "btns" },
        h("button", { class: "btn p", onclick: async () => {
          const out = await save({ ...a, name: name.value, job: job.value, app: app.value, devices: [...picked], blocked: blocked.value.split(",").map((w) => w.trim()).filter(Boolean) });
          if (out) page(out.id, "settings");
        } }, "Save"),
        a.id === "default" ? null : h("button", { class: "btn danger", onclick: async () => {
          if (!confirm(`Delete the agent "${a.name}"? Its schedules and conversations move to "${(byId("default") || { agent: { name: "My agent" } }).agent.name}".`)) return;
          try { await api("/api/agents", { action: "delete", id: a.id }); await loadAgents(); toast("Agent deleted.", true); location.hash = "#/agents/default/chat"; } catch (e) { toast(e.message); }
        } }, "Delete agent"))));
}

function tabSkills(x) {
  return h("section", { class: "grp ag-sec" }, h("h2", { class: "gh" }, "skills"),
    h("div", { class: "ag-pad" }, h("p", {}, `${x.agent.name} can use every installed skill (test techniques, checklists, regression, reports).`),
      h("p", { class: "s muted" }, "Picking skills per agent comes next.")));
}

async function tabSchedules(x, box) {
  box.replaceChildren(h("div", { class: "empty small" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" })));
  let d;
  try { d = await api("/api/schedules"); } catch (e) { box.replaceChildren(h("div", { class: "err", role: "alert" }, e.message)); return; }
  const mine = (d.schedules || []).filter((s) => (s.schedule.agent || "default") === x.agent.id);
  const add = h("button", { class: "btn sm p", onclick: () => { schedForm = { agent: x.agent.id }; location.hash = "#/schedules"; } }, icon("plus", "sm"), "New schedule");
  box.replaceChildren(h("section", { class: "grp ag-sec" }, h("div", { class: "gh" }, `schedules · ${mine.length}`, h("span", { class: "right" }, add)),
    mine.length
      ? h("ul", { class: "ag-rows" }, mine.map((s) => h("li", {}, h("b", {}, s.schedule.name), h("span", {}, s.when_text),
        h("span", { class: "s" }, s.last && s.last.total !== undefined && s.last.total !== null ? `last: ${s.last.ok}/${s.last.total}` : "not run yet"))))
      : h("div", { class: "empty small" }, h("b", {}, "Nothing is scheduled"), h("span", {}, "Ask in Chat, e.g. \"run the checkout tests every 2 hours\", or add one here."))));
}

async function tabLive(x, box) {
  let d;
  try { d = await api("/api/live"); } catch (e) { box.replaceChildren(h("div", { class: "err", role: "alert" }, e.message)); return; }
  const mine = (d.sessions || []).filter((s) => (s.agent || "default") === x.agent.id && s.kind === "mcp");
  box.replaceChildren(h("section", { class: "grp ag-sec" }, h("div", { class: "gh" }, `live · ${mine.length} running`, h("a", { class: "right lk", href: "#/live" }, "Open Live")),
    mine.length
      ? h("ul", { class: "ag-rows" }, mine.map((s) => h("li", {}, h("b", {}, s.id), h("span", {}, s.platform), h("span", { class: "s" }, s.flow ? `${(s.flow.steps || []).length} steps` : ""))))
      : h("div", { class: "empty small" }, h("b", {}, "No tests are running"), h("span", {}, "When this agent runs a test you can watch it here, and take over in Live."))));
}

async function tabHistory(x, box) {
  let d;
  try { d = await api("/api/history"); } catch (e) { box.replaceChildren(h("div", { class: "err", role: "alert" }, e.message)); return; }
  const others = new Set((agData.agents || []).filter((y) => y.agent.id !== x.agent.id).flatMap((y) => y.agent.conversations));
  const mine = x.agent.id === "default"
    ? (d.chats || []).filter((c) => !others.has(c.id))
    : (d.chats || []).filter((c) => x.agent.conversations.includes(c.id));
  box.replaceChildren(h("section", { class: "grp ag-sec" }, h("div", { class: "gh" }, `conversations · ${mine.length}`, h("a", { class: "right lk", href: "#/history" }, "Open History")),
    mine.length
      ? h("ul", { class: "ag-rows" }, mine.map((c) => h("li", {}, h("b", {}, c.title), h("span", { class: "s" }, `${c.messages} messages · ${new Date(c.updated_ms).toLocaleString("en-GB", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" })}`))))
      : h("div", { class: "empty small" }, h("b", {}, "No conversations yet"), h("span", {}, "What you say to this agent in Chat is kept here."))));
}

function page(id, tab) {
  const x = byId(id);
  if (!x) { location.hash = "#/agents/default/chat"; return; }
  const a = x.agent;
  tab = TABS.some(([k]) => k === tab) ? tab : "chat";
  $top().replaceChildren(avatar(x, "lg"), h("div", { class: "ag-head" }, h("h1", { tabindex: "-1" }, a.name),
    h("span", { class: "s" }, `${a.devices.map(deviceName).join(" + ")} · ${modelLabel(a)}`)),
  h("nav", { class: "ag-tabs right", "aria-label": `${a.name}: sections` }, TABS.map(([k, t]) => h("a", { href: `#/agents/${a.id}/${k}`, class: k === tab ? "on" : "", "aria-current": k === tab ? "page" : null }, t))));
  const box = h("div", { class: "ag-main" });
  $body().replaceChildren(h("div", { class: "ag-wrap" }, list(a.id), box));
  if (tab === "chat") {
    if (chatAgent !== a.id || chatHost !== box) { chatData = null; chatSig = ""; }
    chatAgent = a.id;
    chatHost = box;
    if (chatData) renderChat();
    loadChat();
    return;
  }
  chatHost = null;
  if (tab === "instructions") box.replaceChildren(tabInstructions(x));
  else if (tab === "settings") box.replaceChildren(tabSettings(x));
  else if (tab === "skills") box.replaceChildren(tabSkills(x));
  else if (tab === "schedules") tabSchedules(x, box);
  else if (tab === "live") tabLive(x, box);
  else if (tab === "history") tabHistory(x, box);
}

// ---- Create agent (v7 #s4) ----

function pageNew() {
  chatHost = null;
  const d = agDraft || (agDraft = { name: "", job: "", devices: ["web"], app: "", instructions: [], blocked: "" });
  const errs = h("p", { class: "fld-err", role: "alert", hidden: true });
  const name = h("input", { type: "text", maxlength: "40", value: d.name, placeholder: "e.g. Mobile Payments", "aria-label": "Name" });
  name.addEventListener("input", () => { d.name = name.value; });
  const job = h("textarea", { rows: "2", maxlength: "300", placeholder: "e.g. Keep cart and payment working in the Android and iOS app on every release", "aria-label": "What it does" }, d.job);
  job.addEventListener("input", () => { d.job = job.value; });
  const devs = h("div", { class: "chips" }, (agData.devices || []).map((v) => {
    const on = d.devices.includes(v.id);
    const b = h("button", { type: "button", class: "chip pick" + (on ? " on" : ""), "aria-pressed": String(on) }, v.name);
    b.addEventListener("click", () => {
      d.devices = d.devices.includes(v.id) ? d.devices.filter((x) => x !== v.id) : [...d.devices, v.id];
      b.classList.toggle("on"); b.setAttribute("aria-pressed", String(d.devices.includes(v.id)));
    });
    return b;
  }));
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
  const create = async () => {
    errs.hidden = true;
    try {
      const r = await api("/api/agents", { action: "create", agent: { id: "", name: d.name, job: d.job, devices: d.devices, app: d.app, instructions: d.instructions, blocked: d.blocked.split(",").map((w) => w.trim()).filter(Boolean) } });
      agDraft = null;
      await loadAgents();
      toast(`Created "${r.agent.name}".`, true);
      location.hash = `#/agents/${r.agent.id}/chat`;
    } catch (e) { errs.textContent = e.message; errs.hidden = false; errs.focus(); }
  };
  $top().replaceChildren(icon("plus"), h("h1", { tabindex: "-1" }, "New agent"), h("span", { class: "s muted" }, "steps 1–3 are enough to start · or just describe it in Chat"),
    h("span", { class: "right btns tight" }, h("a", { class: "btn", href: "#/agents" }, "Cancel"), h("button", { class: "btn p", onclick: create }, "Create agent")));
  const sec = (n, title, opt, ...kids) => h("section", { class: "grp ag-sec" }, h("h2", { class: "gh" }, `${n} · ${title}`, opt ? h("span", { class: "opt" }, " · optional") : null), h("div", { class: "ag-pad" }, ...kids));
  const toChat = h("button", { class: "btn", onclick: () => {
    location.hash = "#/agents/default/chat";
    setTimeout(() => { const box = document.getElementById("chat-input"); if (box) { box.value = "Create an agent \"Mobile Payments\" for the Android emulator and the iPhone simulator that keeps the cart and payment working, and never presses Pay"; box.focus(); } }, 300);
  } }, icon("chat", "sm"), "Describe it in Chat");
  $body().replaceChildren(h("div", { class: "ag-new" },
    h("div", { class: "ag-form" }, errs,
      sec(1, "who", false, h("label", { class: "fld" }, h("span", { class: "l" }, "Name"), name), h("label", { class: "fld" }, h("span", { class: "l" }, "What it does"), job)),
      sec(2, "where", false, devs, app),
      sec(3, "instructions (how this agent should work)", false, rules, h("div", { class: "add-row" }, rule, h("button", { type: "button", class: "btn", onclick: addRule }, icon("plus", "sm"), "Add")), sugs),
      sec(4, "skills", true, h("p", { class: "s muted" }, "Every installed skill for now; picking skills per agent comes next.")),
      sec(5, "AI model", true, h("p", { class: "s muted" }, "Claude Code with your usual model for now; picking a provider and model per agent comes next.")),
      sec(6, "this agent's limits", true, h("p", { class: "s" }, "It follows your global limits (pay, checkout and delete account are blocked). An agent can only be stricter."),
        h("label", { class: "fld" }, h("span", { class: "l" }, "Extra blocked words for this agent"), blocked))),
    h("aside", { class: "grp ag-side" }, h("h2", { class: "gh" }, "or create it in chat"),
      h("div", { class: "ag-pad" }, h("p", {}, "Tell fastpilot in one sentence what the agent should do. It prepares a card; nothing is created until you click Create."), toChat))));
  name.focus();
}

async function route() {
  // Read again on every visit: an agent may have been made from a Chat card meanwhile.
  if (!agBusy) await loadAgents();
  if (!agData) return;
  const [, , id, tab] = (location.hash || "").split("/");
  if (!id) { location.hash = "#/agents/default/chat"; return; }
  if (id === "new") pageNew(); else page(id, tab);
}

PAGES.agents = () => { route(); };
// Edit on a "New agent" card in Chat: the form, filled in with what the bot proposed.
window.editAgentDraft = (d) => {
  agDraft = { name: d.name || "", job: d.job || "", devices: d.devices || ["web"], app: d.app || "", instructions: d.instructions || [], blocked: (d.blocked || []).join(", ") };
  location.hash = "#/agents/new";
};
})();
