// E4 (v0.7): the first run. Until the first test is saved, the first screen is a short wizard
// (design easy-start-v12 e4a–e4b, e5e; storyboard s7a steps 4–5) and the menu is simple: Chat, Live,
// Device and Settings, the rest under More.
//   1. What do you want to test? Website (paste the address: it is added as you paste, no Next),
//      or an Android or iPhone app (the Add app dialog, which knows about devices).
//   2. What should we check first? One click sends a usual check to Chat, which makes and runs
//      the test; "Something else, in my own words" is the last option.
// "I know my way around" skips it for good; so does sending a check. Uses h(), icon(), api(),
// openChat(), $top(), $body() and UI.* from the other scripts.
(() => {
const DONE = "fastpilot.welcome"; // "step2" after the first app, "done" once skipped or used
const FULL = "fastpilot.fullmenu"; // "1": the person asked for every menu item
const CHECKS = ["Check the login", "Add a product to the cart", "Search for something and open the first result", "Fill in the contact form"];
let info = null; // { apps, tests, at }
let busy = false;
let failures = 0;

async function load() {
  if (busy) return;
  busy = true;
  try {
    const [a, s] = await Promise.all([api("/api/apps"), api("/api/schedules")]);
    info = { apps: a.apps || [], current: a.current, fixed: !!a.fixed_dir, tests: (s.tests || []).length, at: Date.now() };
    failures = 0;
  } catch (_) {
    // Try again a few times before giving up (then: no wizard, the full menu).
    failures += 1;
    if (failures < 4) { busy = false; setTimeout(load, 1500); return; }
    info = { apps: [], tests: 1, at: Date.now() };
  }
  busy = false;
  render();
}

const done = () => localStorage.getItem(DONE) === "done";
const finish = () => localStorage.setItem(DONE, "done");

/// The wizard replaces the first screen while nothing is saved yet and it was not skipped. Until
/// that is known (the first load), the first screen waits too: a Chat page that started drawing
/// meanwhile would draw over the wizard when it finishes (Design Critic, final check M1).
function wants(page) {
  if (done() || page !== "agents") return false;
  if (!info || Date.now() - info.at > 5000) load();
  if (!info) return true;
  // A real first run: no app yet (not a developer's fixed folder), or step 2 of a wizard this
  // browser started.
  return !!info && info.tests === 0 && ((!info.apps.length && !info.fixed) || localStorage.getItem(DONE) === "step2");
}

function header() {
  $top().replaceChildren(h("h1", { tabindex: "-1" }, "Welcome to fastpilot"), h("span", { class: "s muted" }, "first time here"),
    h("span", { class: "right btns tight" }, h("button", { type: "button", class: "btn sm", onclick: () => { finish(); render(); } }, "I know my way around")));
}

function steps(n) {
  // A class per step: the page's CSP allows no inline style (Design Critic, final check L1).
  return h("div", { class: "wz-steps" }, h("span", {}, `Step ${n} of 3`), h("span", { class: `wz-bar step-${n}`, "aria-hidden": "true" }, h("span", {})), h("span", {}, "about 1 minute"));
}

function stepOne() {
  const err = h("p", { class: "err-line", role: "alert", hidden: true });
  const where = h("input", { type: "url", id: "wz-where", autocomplete: "off", spellcheck: "false", placeholder: "https://staging.shop.example", "aria-describedby": "wz-where-h" });
  let adding = false;
  const add = async () => {
    const value = where.value.trim();
    if (!value || adding) return;
    adding = true;
    err.hidden = true;
    try {
      await api("/api/apps", { action: "add", targets: { web: value } });
      localStorage.setItem(DONE, "step2");
      // The app switcher shows the new app at once (L3).
      if (window.Apps) Apps.reload();
      info = null;
      load();
    } catch (e) { err.textContent = e.message; err.hidden = false; where.focus(); }
    adding = false;
  };
  // Pasting an address is the whole step (storyboard s7a step 4): no Next to click.
  where.addEventListener("paste", () => setTimeout(() => { if (/^(https?|file):\/\//i.test(where.value.trim())) add(); }, 0));
  where.addEventListener("keydown", (e) => { if (e.key === "Enter") add(); });
  const kind = (k, t, sub) => h("button", { type: "button", class: "wz-kind", role: "radio", "aria-checked": String(k === "web"),
    onclick: () => { if (k !== "web") { location.hash = `#/apps/add?kind=${k}`; } else where.focus(); } }, h("b", {}, t), h("small", {}, sub));
  return [steps(1), h("h2", { class: "wz-h", tabindex: "-1" }, "What do you want to test?"),
    h("div", { class: "seg wz-kinds", role: "radiogroup", "aria-label": "What do you want to test?" },
      kind("web", "A website", "paste its address"), kind("android", "An Android app", "on the emulator or a phone"), kind("ios", "An iPhone app", "on the simulator or an iPhone")),
    h("label", { class: "fld" }, h("span", { class: "l" }, "Address of the website"), where),
    h("p", { class: "s muted", id: "wz-where-h" }, "Paste it and fastpilot adds it. Use a test or staging site, not your live shop."), err];
}

function stepTwo(app) {
  const send = (t) => { finish(); openChat(t); };
  const own = h("textarea", { rows: "2", maxlength: "600", "aria-label": "What should we check, in your own words", placeholder: "For example: the voucher SAVE10 gives 10% off" });
  return [steps(2), h("h2", { class: "wz-h", tabindex: "-1" }, "What should we check first?"),
    h("p", {}, `Pick one to start on ${app ? app.name : "your app"}. You can say it in your own words instead.`),
    h("div", { class: "wz-checks" }, CHECKS.map((t) => h("button", { type: "button", class: "say-row", onclick: () => send(t) }, h("span", {}, t), h("span", { class: "arrow", "aria-hidden": "true" }, "→")))),
    UI.moreOptions({ label: "Something else, in my own words" }, own, h("div", { class: "btns" }, h("button", { type: "button", class: "btn p", onclick: () => { if (own.value.trim()) send(own.value.trim()); } }, "Send to Chat"))),
    h("p", { class: "s muted" }, "One click makes the test and runs it. If it needs a password, you are asked once, right when it is needed. fastpilot shows the steps first, and nothing is saved until you say so.")];
}

function page() {
  if (!info) {
    $top().replaceChildren(h("h1", { tabindex: "-1" }, "Chat"));
    $body().replaceChildren(h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Loading" }), h("b", {}, "Getting ready…")));
    return;
  }
  header();
  const app = info.apps.find((a) => a.id === info.current) || info.apps[0] || null;
  $body().replaceChildren(h("section", { class: "wz", "aria-label": "First test" }, ...(info.apps.length ? stepTwo(app) : stepOne())));
  document.querySelector(".wz-h")?.focus();
}

/// The simple menu until the first saved test (or "Show all menus" in Settings).
function menu() {
  const simple = localStorage.getItem(FULL) !== "1" && !!info && !info.fixed && info.tests === 0;
  document.body.classList.toggle("simple-menu", simple);
  const rail = document.querySelector(".rail");
  if (rail && !rail.querySelector(".rail-more")) {
    const b = h("button", { type: "button", class: "rail-more", "aria-expanded": "false", onclick: () => {
      const open = document.body.classList.toggle("menu-open");
      b.setAttribute("aria-expanded", String(open));
    } }, icon("chev"), h("small", {}, "More"));
    rail.querySelector(".sp")?.before(b);
  }
  if ((!info || Date.now() - info.at > 5000) && !busy) load();
}

function fullMenuToggle() {
  const on = localStorage.getItem(FULL) === "1";
  return h("label", { class: "s wz-full" }, h("input", { type: "checkbox", checked: on, onchange: (e) => { localStorage.setItem(FULL, e.target.checked ? "1" : "0"); render(); } }), " Show all menus");
}

window.Welcome = { wants, page, menu, fullMenuToggle, refresh: () => { info = null; load(); } };
})();
