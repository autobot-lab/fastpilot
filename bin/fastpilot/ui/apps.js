// fastpilot ui: Apps (v0.6, spec SPEC-apps-jira.md, design apps-jira-v10). What fastpilot is
// connected to: Jira (connected only from this page, by a click), the schedules' Slack/webhook
// notifications, GitHub pull request comments, and what comes later. Uses h(), icon(), api(),
// toast() from app.js. Values from the server are set as text, never as HTML. Everything is inside
// one function; only PAGES.apps is shared.
(() => {
"use strict";

let appsData = null; // { jira, webhook_schedules, token_kept }
let appsBusy = false;
let githubOpen = false;

async function loadApps() {
  if (appsBusy) return;
  appsBusy = true;
  try {
    appsData = await api("/api/apps");
  } catch (e) {
    appsData = { error: e.message };
  }
  appsBusy = false;
  if ((location.hash || "").startsWith("#/apps")) render();
}

const badge = (letter, kind) => h("span", { class: "app-badge " + kind, "aria-hidden": "true" }, letter);
const pill = (text, kind) => h("span", { class: "app-pill " + kind }, kind === "bad" ? "! " : "● ", text);

function jiraCard(j) {
  if (!j) {
    return h("div", { class: "app-card wide", "data-app": "jira" },
      h("div", { class: "app-head" }, badge("J", "jira"), h("div", {}, h("b", {}, "Jira"),
        h("p", {}, "Turn failed tests into Jira issues, add results to a ticket, and write tests from a ticket. You approve everything first."))),
      h("div", { class: "app-acts" }, h("a", { class: "btn p", href: "#/apps/jira" }, "Connect Jira")),
      h("p", { class: "s" }, "Works with Jira Cloud addresses that end in .atlassian.net. Jira Server and Data Center are not supported yet."));
  }
  const project = j.project_name ? `Project ${j.project_name}${j.project_key ? ` (${j.project_key})` : ""}` : "";
  return h("div", { class: "app-card" + (j.needs_attention ? " attention" : ""), "data-app": "jira" },
    h("div", { class: "app-head" }, badge("J", "jira"), h("div", {}, h("b", {}, "Jira"),
      h("p", {}, j.needs_attention ? "Jira no longer accepts your token. Nothing was lost." : "Send failed tests to Jira as issues or comments.")),
      j.needs_attention ? pill("Needs attention", "bad") : pill("Connected", "ok")),
    h("p", { class: "s" }, [j.site, project, j.email].filter(Boolean).join(" · ")),
    h("div", { class: "app-acts" },
      j.needs_attention ? h("a", { class: "btn p", href: "#/apps/jira" }, "Reconnect") : null,
      h("a", { class: "btn", href: "#/apps/jira" }, "Manage"),
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

function pageApps() {
  $top().replaceChildren(icon("plug"), h("h1", { tabindex: "-1" }, "Apps"), h("span", { class: "s muted" }, "Connect fastpilot to the tools your team already uses"));
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
  $body().replaceChildren(h("div", { class: "apps" },
    h("div", { class: "apps-main" },
      section("Connected", connected.length ? [h("div", { class: "app-grid" }, connected)]
        : [h("div", { class: "app-none" }, h("b", {}, "Nothing is connected yet"),
          h("p", {}, "Connect Jira to send failed tests to your team's board. Slack and webhook notifications you set on a schedule show up here too."))]),
      section("Available", [h("div", { class: "app-grid" }, available)]),
      section("Coming soon", [h("div", { class: "app-soon-row" }, soon)])),
    h("aside", { class: "apps-side", "aria-label": "How Jira works with fastpilot" },
      h("h2", { class: "app-sec-t" }, "How Jira works with fastpilot"),
      h("ol", { class: "app-how" },
        h("li", {}, h("b", {}, "You connect once"), h("p", {}, "Your Jira address, email and a token from Atlassian, here on this page. Takes about 2 minutes.")),
        h("li", {}, h("b", {}, "You ask, you approve"), h("p", {}, "Ask the bot or Claude Code: “make a Jira ticket for this”. You always see the exact text first, and nothing is sent until you click.")),
        h("li", {}, h("b", {}, "Read a ticket, test it"), h("p", {}, "Say “test KAN-12” in Chat. fastpilot reads the ticket and proposes tests. You can then add the result to the ticket as a comment."))),
      h("div", { class: "app-note info" }, h("b", {}, "Coming soon: "), "when someone moves a ticket to QA, fastpilot tests it by itself. For now, every test starts with you."),
      h("div", { class: "app-note ok" }, `Your Jira token is stored in ${kept} and is never shown again. Jira's text can never change your limits.`))));
}

PAGES.apps = (sub) => {
  if (sub === "jira" && PAGES.appsJira) return PAGES.appsJira();
  pageApps();
};
PAGES.appsReload = () => { appsData = null; };
// Read again on every visit: a schedule or the connection may have changed meanwhile.
window.addEventListener("hashchange", () => {
  if (location.hash === "#/apps") { appsData = null; loadApps(); }
});
})();
