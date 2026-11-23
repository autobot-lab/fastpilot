// fastpilot ui: Apps (v0.6, spec SPEC-apps-jira.md, design apps-jira-v10). What fastpilot is
// connected to: Jira (connected only from this page, by a click), the schedules' Slack/webhook
// notifications, GitHub pull request comments, and what comes later. Uses h(), icon(), api(),
// toast() from app.js. Values from the server are set as text, never as HTML. Everything is inside
// one function; only PAGES.integrations is shared.
(() => {
"use strict";

let appsData = null; // { jira, webhook_schedules, token_kept }
let appsBusy = false;
let githubOpen = false;

async function loadApps() {
  if (appsBusy) return;
  appsBusy = true;
  try {
    appsData = await api("/api/integrations");
  } catch (e) {
    appsData = { error: e.message };
  }
  appsBusy = false;
  if ((location.hash || "").startsWith("#/integrations")) render();
}

const badge = (letter, kind) => h("span", { class: "app-badge " + kind, "aria-hidden": "true" }, letter);
const pill = (text, kind) => h("span", { class: "app-pill " + kind }, kind === "bad" ? "! " : "● ", text);

function jiraCard(j) {
  if (!j) {
    return h("div", { class: "app-card wide", "data-app": "jira" },
      h("div", { class: "app-head" }, badge("J", "jira"), h("div", {}, h("b", {}, "Jira"),
        h("p", {}, "Turn failed tests into Jira issues, add results to a ticket, and write tests from a ticket. You approve everything first."))),
      h("div", { class: "app-acts" }, h("a", { class: "btn p", href: "#/integrations/jira" }, "Connect Jira")),
      h("p", { class: "s" }, "Works with Jira Cloud addresses that end in .atlassian.net. Jira Server and Data Center are not supported yet."));
  }
  const project = j.project_name ? `Project ${j.project_name}${j.project_key ? ` (${j.project_key})` : ""}` : "";
  return h("div", { class: "app-card" + (j.needs_attention ? " attention" : ""), "data-app": "jira" },
    h("div", { class: "app-head" }, badge("J", "jira"), h("div", {}, h("b", {}, "Jira"),
      h("p", {}, j.needs_attention ? "Jira no longer accepts your token. Nothing was lost." : "Send failed tests to Jira as issues or comments.")),
      j.needs_attention ? pill("Needs attention", "bad") : pill("Connected", "ok")),
    h("p", { class: "s" }, [j.site, project, j.email].filter(Boolean).join(" · ")),
    h("div", { class: "app-acts" },
      j.needs_attention ? h("a", { class: "btn p", href: "#/integrations/jira" }, "Reconnect") : null,
      h("a", { class: "btn", href: "#/integrations/jira" }, "Manage"),
      j.checked_ms ? h("span", { class: "s" }, "Last checked " + when(j.checked_ms)) : null));
}

function when(ms) {
  const d = new Date(ms);
  const today = new Date().toDateString() === d.toDateString();
  const t = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return today ? `today, ${t}` : `${d.toLocaleDateString([], { day: "numeric", month: "short" })}, ${t}`;
}

function slackCard(n) {
  return h("div", { class: "app-card", "data-app": "slack" },
    h("div", { class: "app-head" }, badge("S", "slack"), h("div", {}, h("b", {}, "Slack or webhook"),
      h("p", {}, "Get a message when a scheduled test fails.")), n ? pill("Connected", "ok") : null),
    n ? h("p", { class: "s" }, `Used by ${n} ${n === 1 ? "schedule" : "schedules"}`) : null,
    h("div", { class: "app-acts" }, h("a", { class: "btn", href: "#/schedules" }, n ? "Manage" : "Set up"),
      h("span", { class: "s" }, "Opens the schedules, where each one has its own notification.")));
}

function githubCard() {
  return h("div", { class: "app-card", "data-app": "github" },
    h("div", { class: "app-head" }, badge("G", "github"), h("div", {}, h("b", {}, "GitHub"),
      h("p", {}, "Post the result of a run as a comment on a pull request."))),
    h("div", { class: "app-acts" }, h("button", { class: "btn", "aria-expanded": String(githubOpen), onclick: () => { githubOpen = !githubOpen; render(); } }, githubOpen ? "Hide the steps" : "See how")),
    githubOpen ? h("ol", { class: "app-steps" },
      h("li", {}, "Nothing to connect here: it runs in GitHub Actions, with GitHub's own sign-in for the workflow."),
      h("li", {}, "Add the fastpilot GitHub Action (autobot-lab/fastpilot-action) to a workflow, or run fastpilot run with the pull request option there."),
      h("li", {}, "fastpilot comments on the pull request with the result: one comment, updated on each run.")) : null);
}

// Security (Medium 1): Jira issues and comments Claude Code asked for. Nothing was sent; the
// person sends each one here (or with `fastpilot jira approve`). The text is exactly what is sent.
let reqBusy = false;
async function decideRequest(id, approve) {
  if (reqBusy) return;
  reqBusy = true;
  try {
    const r = await api("/api/jira", { action: approve ? "approve" : "decline", id });
    toast(r.message, true);
  } catch (e) { toast(e.message); }
  reqBusy = false;
  appsData = null;
  loadApps();
}

function requestCard(r) {
  const issue = r.kind === "issue";
  const mins = Math.max(0, Math.round((r.expires_ms - Date.now()) / 60000));
  return h("section", { class: "app-card req", "aria-label": r.title },
    h("div", { class: "app-head" }, badge("J", "jira"),
      h("div", {}, h("b", {}, r.title), h("p", { class: "s" }, `Asked by Claude Code · ${r.site} · expires in ${mins} min`))),
    h("dl", { class: "prop-rows" },
      issue ? [h("dt", {}, "Summary"), h("dd", {}, r.summary)] : null,
      h("dt", {}, issue ? "Description" : "Comment"), h("dd", {}, h("pre", { class: "req-text" }, r.text || "(none)"))),
    h("p", { class: "s" }, "Did you ask for this? Exactly this text is sent. It never changes a ticket's status."),
    h("div", { class: "app-acts" },
      h("button", { class: "btn p", onclick: () => decideRequest(r.id, true) }, issue ? "Create issue" : "Add comment"),
      h("button", { class: "btn", onclick: () => decideRequest(r.id, false) }, "Don't send")));
}

// QA re-test Low 8: requests can be sent or dropped elsewhere (the terminal, another tab): while
// Apps is open, the list is read again every few seconds and redrawn when it changed.
let appsPoll = null;
function pollApps() {
  clearTimeout(appsPoll);
  appsPoll = setTimeout(async () => {
    if ((location.hash || "") !== "#/integrations") return;
    try {
      const fresh = await api("/api/integrations");
      const ids = (d) => JSON.stringify(((d && d.jira_requests) || []).map((r) => r.id));
      if (ids(fresh) !== ids(appsData)) { appsData = fresh; render(); return; }
    } catch (_) { /* the next try */ }
    pollApps();
  }, 3000);
}

function pageApps() {
  pollApps();
  $top().replaceChildren(icon("plug"), h("h1", { tabindex: "-1" }, "Integrations"), h("span", { class: "s muted" }, "Connect fastpilot to the tools your team already uses"));
  if (!appsData) {
    if (!appsBusy) loadApps();
    $body().replaceChildren(h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" }), h("b", {}, "Reading apps…")));
    return;
  }
  if (appsData.error) {
    $body().replaceChildren(h("div", { class: "err", role: "alert" }, "Could not read the apps: " + appsData.error));
    return;
  }
  const j = appsData.jira;
  const n = appsData.webhook_schedules || 0;
  const connected = [j ? jiraCard(j) : null, n ? slackCard(n) : null].filter(Boolean);
  const available = [j ? null : jiraCard(null), n ? null : slackCard(0), githubCard()].filter(Boolean);
  const soon = ["Linear", "Telegram", "Confluence", "Email"].map((name) => h("div", { class: "app-soon" }, badge(name[0], "soon"), h("b", {}, name)));
  const section = (title, kids) => h("section", { class: "app-sec", "aria-label": title }, h("h2", { class: "app-sec-t" }, title), ...kids);
  const kept = appsData.token_kept || "this computer's key store";
  const reqs = appsData.jira_requests || [];
  $body().replaceChildren(h("div", { class: "apps" },
    h("div", { class: "apps-main" },
      reqs.length ? section("Waiting for your OK", [h("div", { class: "app-grid" }, reqs.map(requestCard))]) : null,
      section("Connected", connected.length ? [h("div", { class: "app-grid" }, connected)]
        : [h("div", { class: "app-none" }, h("b", {}, "Nothing is connected yet"),
          h("p", {}, "Connect Jira to send failed tests to your team's board. Slack and webhook notifications you set on a schedule show up here too."))]),
      section("Available", [h("div", { class: "app-grid" }, available)]),
      section("Coming soon", [h("div", { class: "app-soon-row" }, soon)]),
      sayIt(j ? ["File the last failure in Jira"] : ["Connect Jira"])),
    h("aside", { class: "apps-side", "aria-label": "How Jira works with fastpilot" },
      h("h2", { class: "app-sec-t" }, "How Jira works with fastpilot"),
      h("ol", { class: "app-how" },
        h("li", {}, h("b", {}, "You connect once"), h("p", {}, "Sign in with Atlassian and pick your site. No token to create.")),
        h("li", {}, h("b", {}, "You ask, you approve"), h("p", {}, "Ask the bot or Claude Code: “make a Jira ticket for this”. You always see the exact text first, and nothing is sent until you click.")),
        h("li", {}, h("b", {}, "Read a ticket, test it"), h("p", {}, "Say “test KAN-12” in Chat. fastpilot reads the ticket and proposes tests. You can then add the result to the ticket as a comment."))),
      h("div", { class: "app-note ok" }, `Your Jira sign-in is stored in ${kept} and is never shown. Jira's text can never change your limits.`))));
}

// ---- Apps › Connect Jira / Manage (design 02) ----
let jiraState = null; // { connected, settings, kept_in }
let jf = null; // the form: { site, email, token, project, replacing, useToken }
let jt = null; // the last test: { kind, message, ok, name, projects, wait }
let jProjects = null; // E5: the signed-in account's projects, read when the page shows them
let jBusy = false;
let jWaitTimer = null;

async function jiraPost(body) {
  const r = await fetch("/api/jira", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  let v = null;
  try { v = await r.json(); } catch (_) { v = null; }
  if (!r.ok) { const e = new Error((v && v.error) || "Something went wrong."); e.kind = v && v.kind; e.wait = v && v.wait; throw e; }
  return v;
}

async function loadJira() {
  try { jiraState = await api("/api/jira"); } catch (e) { jiraState = { error: e.message }; }
  const s = jiraState.settings || {};
  jf = { site: s.site || "", email: s.email || "", token: "", project: s.project_key || "", replacing: false, useToken: jiraState.connected && s.auth !== "atlassian" };
  jt = jiraState.connected && s.display_name ? { ok: true, name: s.display_name, projects: null, saved: true } : null;
  jProjects = null;
  if ((location.hash || "").startsWith("#/integrations/jira")) PAGES.appsJira();
}

const creds = () => ({ site: jf.site, email: jf.email, token: jf.replacing || !(jiraState && jiraState.connected) ? jf.token : "" });
const tokenNeeded = () => jf.replacing || !(jiraState && jiraState.connected);

async function testConnection(again) {
  if (jBusy) return;
  clearTimeout(jWaitTimer);
  jBusy = true; jt = { kind: "testing" };
  PAGES.appsJira();
  try {
    const r = await jiraPost({ action: "test", ...creds() });
    jt = { ok: true, name: r.display_name, projects: r.projects };
    jf.site = r.site;
    // QA bug 3: a new token for the same account keeps the saved default project.
    const savedProject = ((jiraState && jiraState.settings) || {}).project_key;
    if (!jf.project && savedProject && r.projects.some((p) => p.key === savedProject)) jf.project = savedProject;
    if (!r.projects.some((p) => p.key === jf.project)) jf.project = r.projects.length === 1 ? r.projects[0].key : "";
  } catch (e) {
    jt = { kind: e.kind || "other", message: e.message, wait: e.wait };
    if (e.kind === "busy" && !again) {
      // Jira asked us to wait: one automatic try after the wait, then it says it is busy.
      let left = Math.min(e.wait || 20, 120);
      jt.left = left;
      const tick = () => { left -= 1; jt.left = left; if (left <= 0) testConnection(true); else { jWaitTimer = setTimeout(tick, 1000); PAGES.appsJira(); } };
      jWaitTimer = setTimeout(tick, 1000);
    }
  }
  jBusy = false;
  PAGES.appsJira();
}

async function saveJira() {
  jBusy = true; PAGES.appsJira();
  try {
    const r = await jiraPost({ action: "save", ...creds(), project: jf.project });
    toast(r.ok, true);
    PAGES.appsReload();
    jiraState = null;
    location.hash = "#/integrations";
  } catch (e) {
    jt = { kind: e.kind || "other", message: e.message };
  }
  jBusy = false;
  if ((location.hash || "").startsWith("#/integrations/jira")) PAGES.appsJira();
}

function confirmDisconnect() {
  const site = (jiraState.settings || {}).site || "";
  return new Promise((resolve) => {
    const done = (v) => { dlg.close(); dlg.remove(); resolve(v); };
    const keep = h("button", { class: "btn", onclick: () => done(false) }, "Keep connected");
    const dlg = h("dialog", { class: "hist-dlg", "aria-labelledby": "jira-dc-t" },
      h("h2", { id: "jira-dc-t" }, "Disconnect Jira?"),
      h("p", {}, `fastpilot will forget your token and settings for ${site}. Tickets already created stay in Jira. The bot and Claude Code cannot send to Jira until you connect again.`),
      h("div", { class: "btns tight" }, keep, h("button", { class: "btn danger", onclick: () => done(true) }, "Disconnect")));
    dlg.addEventListener("cancel", (e) => { e.preventDefault(); done(false); });
    document.body.append(dlg);
    dlg.showModal();
    keep.focus();
  });
}

async function disconnectJira() {
  if (!(await confirmDisconnect())) return;
  try { toast((await jiraPost({ action: "disconnect" })).ok, true); } catch (e) { toast(e.message); }
  PAGES.appsReload();
  jiraState = null;
  location.hash = "#/integrations";
}

const RED = ["refused", "no_access"];
function testResult() {
  if (!jt) return null;
  if (jt.kind === "testing") return h("div", { class: "jt info", role: "status" }, `Checking ${jf.site || "Jira"}… This takes a few seconds.`);
  if (jt.ok) {
    const n = jt.projects ? jt.projects.length : null;
    if (n === 0) return h("div", { class: "jt warn", role: "alert" }, `! Connected as ${jt.name}, but no projects were found. Your account can sign in but cannot see any project. Ask your Jira admin to add you to one. Save stays off.`);
    return h("div", { class: "jt ok", role: "status" }, "✓ Connected as ", h("b", {}, jt.name), n === null ? "" : ` · ${n} ${n === 1 ? "project" : "projects"} found`);
  }
  if (jt.kind === "busy") {
    return h("div", { class: "jt info", role: "status" }, jt.left > 0 ? `Jira asked us to wait. Trying again in ${jt.left} seconds. ` : "Jira is busy right now. Try again in a minute. ",
      jt.left > 0 ? h("button", { class: "btn sm", onclick: () => { clearTimeout(jWaitTimer); testConnection(true); } }, "Try now") : null);
  }
  const cls = RED.includes(jt.kind) ? "bad" : "warn";
  const kids = [(cls === "bad" ? "✗ " : "! ") + jt.message + (jt.kind === "refused" ? " Nothing was saved." : "")];
  if (jt.kind === "no_access") {
    kids.push(" ", h("button", { class: "btn sm", onclick: () => copyText(`Hi, please allow me to use Atlassian API tokens for Jira on ${jf.site}, so I can connect fastpilot (a testing tool) to my account. Thank you.`, "Message") }, "Copy a message for your admin"));
  }
  return h("div", { class: "jt " + cls, role: "alert" }, ...kids);
}

function field(label, input, hint, id) {
  return h("label", { class: "fld" }, h("span", { class: "l" }, label), input, hint ? h("span", { class: "s", id }, hint) : null);
}

// ---- E5: Sign in with Atlassian (no token) ----
async function signIn() {
  if (jBusy) return;
  jBusy = true; jt = null; PAGES.appsJira();
  try {
    const r = await jiraPost({ action: "connect" });
    // Atlassian's own sign-in page; it comes back to this dashboard.
    location.assign(r.url);
    return;
  } catch (e) { jt = { kind: e.kind || "other", message: e.message }; }
  jBusy = false; PAGES.appsJira();
}

async function jiraChoose(action, key, value) {
  if (jBusy) return;
  jBusy = true; PAGES.appsJira();
  try { toast((await jiraPost({ action, [key]: value })).ok, true); jiraState = null; PAGES.appsReload(); }
  catch (e) { toast(e.message); }
  jBusy = false; PAGES.appsJira();
}

async function loadProjects() {
  try { jProjects = (await jiraPost({ action: "projects" })).projects; } catch (e) { jProjects = { error: e.message }; }
  PAGES.appsJira();
}

function pageSignIn(signedIn) {
  const s = jiraState.settings || {};
  const choose = jiraState.choose_site || [];
  const failed = (location.hash || "").endsWith("/failed");
  $top().replaceChildren(icon("plug"), h("div", {}, h("a", { class: "lk s", href: "#/integrations" }, "Integrations /"), h("h1", { tabindex: "-1" }, signedIn ? "Jira" : "Connect Jira")),
    h("span", { class: "s muted" }, signedIn ? "Connected" : "one click"),
    h("span", { class: "right btns tight" }, h("a", { class: "btn", href: "#/integrations" }, "Back to Integrations")));
  let main;
  if (signedIn) {
    if (!jProjects) loadProjects();
    const list = Array.isArray(jProjects) ? jProjects : [];
    const proj = h("select", { class: "sel", "aria-label": "Default project", disabled: !list.length || jBusy },
      list.length ? null : h("option", { value: "" }, s.project_key ? `${s.project_name} (${s.project_key})` : "Reading projects…"),
      list.map((p) => h("option", { value: p.key, selected: p.key === s.project_key }, `${p.name} (${p.key})`)));
    proj.addEventListener("change", () => jiraChoose("project", "project", proj.value));
    main = [
      h("section", { class: "grp jira-sec" }, h("h2", { class: "gh" }, "your jira account"),
        h("div", { class: "ag-pad" }, h("div", { class: "jt ok", role: "status" }, "✓ Connected as ", h("b", {}, s.display_name || "you"), ` · ${s.site}`),
          h("p", { class: "s muted" }, "Signed in with Atlassian. Nothing to renew: fastpilot keeps the sign-in fresh by itself."))),
      h("section", { class: "grp jira-sec" }, h("h2", { class: "gh" }, "where tickets go"),
        h("div", { class: "form ag-pad" }, field("Default project", proj, null), h("span", { class: "s" }, jProjects && jProjects.error ? jProjects.error : "Used when the bot or Claude Code files a bug. You can name another project each time."))),
      h("section", { class: "grp jira-sec danger-zone" }, h("h2", { class: "gh" }, "disconnect"),
        h("div", { class: "ag-pad" }, h("p", {}, "fastpilot's access is revoked at Atlassian and forgotten here. Tickets already created stay in Jira."),
          h("button", { class: "btn danger", onclick: disconnectJira }, "Disconnect Jira…"))),
    ];
  } else if (choose.length) {
    main = [h("section", { class: "grp jira-sec" }, h("h2", { class: "gh" }, "which jira site?"),
      h("div", { class: "ag-pad" }, h("p", {}, "Your Atlassian account can use more than one site. Pick the one fastpilot files bugs in."),
        h("div", { class: "btns" }, choose.map((c) => h("button", { class: "btn", disabled: jBusy, onclick: () => jiraChoose("site", "site", c.site) }, `Use ${c.name || c.site}`, h("span", { class: "s muted" }, ` · ${c.site}`))))))];
  } else {
    main = [h("section", { class: "grp jira-sec" }, h("h2", { class: "gh" }, "connect jira"),
      h("div", { class: "ag-pad" },
        failed && !jt ? h("div", { class: "jt warn", role: "alert" }, "! The last sign-in did not finish. Nothing was saved. Try again.") : null,
        jt && jt.message ? h("div", { class: "jt warn", role: "alert" }, "! " + jt.message) : null,
        h("p", {}, "You sign in on Atlassian's page and pick your site. No token to create, nothing to copy."),
        h("button", { class: "btn p", disabled: jBusy, onclick: signIn }, icon("plug", "sm"), jBusy ? "Opening Atlassian…" : "Sign in with Atlassian"),
        h("p", { class: "s muted" }, h("button", { class: "lk", type: "button", onclick: () => { jf.useToken = true; jt = null; PAGES.appsJira(); } }, "Use an API token instead"), " (if your admin blocks Atlassian sign-in for apps)")))];
  }
  $body().replaceChildren(h("div", { class: "apps" },
    h("div", { class: "apps-main" }, ...main),
    h("aside", { class: "apps-side" },
      h("h2", { class: "app-sec-t" }, "What happens"),
      h("ol", { class: "app-how" },
        h("li", {}, h("b", {}, "You sign in at Atlassian"), h("p", {}, "fastpilot never sees your password.")),
        h("li", {}, h("b", {}, "The sign-in is kept safe"), h("p", {}, `In ${jiraState.kept_in || "this computer's key store"}, only on this computer. Disconnect revokes it.`)),
        h("li", {}, h("b", {}, "You approve every write"), h("p", {}, "The bot and Claude Code file bugs and comments only after your click."))),
      h("div", { class: "app-note warn" }, h("b", {}, "Jira Cloud only. "), "Sites that end in .atlassian.net."))));
}

function pageJira() {
  if (!jiraState) { loadJira(); $body().replaceChildren(h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" }), h("b", {}, "Reading the Jira connection…"))); return; }
  if (jiraState.error) { $body().replaceChildren(h("div", { class: "err", role: "alert" }, "Could not read the Jira connection: " + jiraState.error)); return; }
  const connected = jiraState.connected;
  const signedIn = connected && (jiraState.settings || {}).auth === "atlassian";
  if (signedIn || !jf.useToken) return pageSignIn(signedIn);
  const kept = jiraState.kept_in || "this computer's key store";
  const clear = () => { clearTimeout(jWaitTimer); if (jt && !jt.saved) jt = null; else if (jt && jt.saved) jt = null; };
  const input = (key, attrs) => {
    const el = h("input", { ...attrs, value: jf[key] });
    // QA bug 2: leaving a field must not redraw the form, or the next click or focus lands on an
    // element that was just replaced (the first Test connection press was lost).
    el.addEventListener("input", () => { jf[key] = el.value; clear(); redrawControls(); });
    return el;
  };
  const site = input("site", { type: "text", autocomplete: "off", spellcheck: "false", placeholder: "yourteam.atlassian.net", "aria-describedby": "jira-site-h" });
  const email = input("email", { type: "email", autocomplete: "username", placeholder: "you@yourteam.com", "aria-describedby": "jira-email-h" });
  let tokenRow;
  if (tokenNeeded()) {
    const token = input("token", { type: "password", autocomplete: "off", spellcheck: "false", "aria-describedby": "jira-token-h" });
    tokenRow = field("API token", token, null);
    tokenRow.append(h("span", { class: "s", id: "jira-token-h" }, connected ? "Paste the new token. The old one is kept until you press Save changes. " : "A token acts like a password. ",
      h("a", { class: "lk", href: "https://id.atlassian.com/manage-profile/security/api-tokens", target: "_blank", rel: "noopener noreferrer" }, "Create a token in Atlassian"), " (opens in a new tab)"));
  } else {
    tokenRow = h("div", { class: "fld" }, h("span", { class: "l" }, "API token"),
      h("div", { class: "jira-saved" }, h("span", { class: "mono", "aria-hidden": "true" }, "••••••••"), h("span", {}, kept ? `Saved in ${kept}. It is never shown again.` : "Saved. It is never shown again."),
        h("button", { class: "btn sm", onclick: () => { jf.replacing = true; jf.token = ""; jt = null; PAGES.appsJira(); } }, "Replace token")));
  }
  const filled = jf.site.trim() && jf.email.trim() && (!tokenNeeded() || jf.token.trim());
  const testBtn = h("button", { class: "btn p", disabled: !filled || jBusy, onclick: () => testConnection(false), "aria-describedby": "jira-test-why" }, "Test connection");
  const why = h("span", { class: "s", id: "jira-test-why" }, filled ? "" : `Enter your ${!jf.site.trim() ? "Jira address" : !jf.email.trim() ? "email" : "API token"}. Test connection stays off until all three fields are filled.`);
  const projects = (jt && jt.ok && jt.projects) || null;
  const proj = h("select", { class: "sel", "aria-label": "Default project", disabled: !projects || !projects.length },
    h("option", { value: "" }, projects ? "Choose a project" : connected ? `${(jiraState.settings || {}).project_name || ""} (${jf.project})` : "Test the connection first"),
    (projects || []).map((p) => h("option", { value: p.key, selected: p.key === jf.project }, `${p.name} (${p.key})`)));
  if (!projects && connected && jf.project) proj.value = "";
  proj.addEventListener("change", () => { jf.project = proj.value; PAGES.appsJira(); });
  const canSave = jt && jt.ok && jt.projects && jt.projects.length && jf.project && !jBusy;
  const saveBtn = h("button", { class: "btn p", disabled: !canSave, onclick: saveJira }, connected ? "Save changes" : "Connect Jira");
  const pastedText = () => (/^https?:\/\//i.test(jf.site) || jf.site.includes("/") ? "We use only the address and ignore the rest of what you paste." : "");
  const pasted = h("span", { class: "s", id: "jira-site-paste" }, pastedText());
  $top().replaceChildren(icon("plug"), h("div", {}, h("a", { class: "lk s", href: "#/integrations" }, "Integrations /"), h("h1", { tabindex: "-1" }, connected ? "Jira · Manage" : "Connect Jira")),
    h("span", { class: "s muted" }, connected ? "Connected" : "about 2 minutes"),
    h("span", { class: "right btns tight" }, h("a", { class: "btn", href: "#/integrations" }, connected ? "Back to Integrations" : "Cancel"), saveBtn));
  function redrawControls() {
    const ok = jf.site.trim() && jf.email.trim() && (!tokenNeeded() || jf.token.trim());
    testBtn.disabled = !ok || jBusy;
    pasted.textContent = pastedText();
    if (ok) why?.replaceChildren();
    else if (why) why.textContent = `Enter your ${!jf.site.trim() ? "Jira address" : !jf.email.trim() ? "email" : "API token"}. Test connection stays off until all three fields are filled.`;
    saveBtn.disabled = !(jt && jt.ok && jt.projects && jt.projects.length && jf.project && !jBusy);
    const r = document.getElementById("jira-result");
    if (r) r.replaceChildren(...[testResult()].filter(Boolean));
  }
  $body().replaceChildren(h("div", { class: "apps" },
    h("div", { class: "apps-main" },
      h("section", { class: "grp jira-sec" }, h("h2", { class: "gh" }, connected ? "your jira account" : "1 · your jira account"),
        h("div", { class: "form ag-pad" },
          field("Jira address", site, null), h("span", { class: "s", id: "jira-site-h" }, "The part of your Jira web address before .atlassian.net. You can paste the whole address."), pasted,
          h("div", { class: "jira-two" }, h("div", {}, field("Email", email, null), h("span", { class: "s", id: "jira-email-h" }, "The email you use to sign in to Atlassian.")), tokenRow),
          h("div", { class: "jira-test" }, testBtn, why, h("div", { id: "jira-result" }, ...[testResult()].filter(Boolean))))),
      h("section", { class: "grp jira-sec" }, h("h2", { class: "gh" }, connected ? "where tickets go" : "2 · where tickets go"),
        h("div", { class: "form ag-pad" },
          h("div", {}, field("Default project", proj, null), h("span", { class: "s" }, "Used when the bot or Claude Code files a bug. You can name another project each time.")))),
      connected ? h("section", { class: "grp jira-sec danger-zone" }, h("h2", { class: "gh" }, "disconnect"),
        h("div", { class: "ag-pad" }, h("p", {}, "fastpilot forgets the token and these settings. Tickets already created stay in Jira. You can connect again whenever you like."),
          h("button", { class: "btn danger", onclick: disconnectJira }, "Disconnect Jira…"))) : null,
      h("p", { class: "s muted" }, `Saving stores your token in ${kept || "this computer's key store"}. Connecting, changing and disconnecting can only be done on this page, never from Chat.`)),
    h("aside", { class: "apps-side" },
      h("h2", { class: "app-sec-t" }, "What happens when you press Save"),
      h("ol", { class: "app-how" },
        h("li", {}, h("b", {}, "The token is kept safe"), h("p", {}, `In ${kept || "this computer's key store"}, only on this computer. It is never shown again, not even here.`)),
        h("li", {}, h("b", {}, "The address, email and project are saved"), h("p", {}, "No secrets in them.")),
        h("li", {}, h("b", {}, "Apps shows Jira as Connected"), h("p", {}, "Then the bot and Claude Code can file bugs and comments, always after your OK."))),
      h("div", { class: "app-note warn" }, h("b", {}, "Jira Cloud only. "), "The address must end in .atlassian.net. Jira Server and Data Center are not supported yet."),
      h("div", { class: "app-note info" }, h("b", {}, "Save is off until the test passes"), " and a project is chosen. Change any field after testing and the test result clears."))));
}

PAGES.integrations = (sub) => {
  if (sub === "jira") return pageJira();
  pageApps();
};
PAGES.appsReload = () => { appsData = null; };
PAGES.appsJira = () => pageJira();
// Read again on every visit: a schedule or the connection may have changed meanwhile.
window.addEventListener("hashchange", () => {
  if (location.hash === "#/integrations") { appsData = null; loadApps(); }
  if (location.hash === "#/integrations/jira") { jiraState = null; clearTimeout(jWaitTimer); }
});
})();
