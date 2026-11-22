// fastpilot ui: shared building blocks (UXA4, v0.7). One look and one behaviour on every page:
// PageHeader, MoreOptions, ConfirmDialog, Empty, Notice. Built with app.js's h() and icon(); text
// is always set as text, never as HTML. Exposed as one global, UI.
(() => {
"use strict";

/// The children worth drawing: arrays flattened; null, undefined, false and "" dropped. A null
/// handed to replaceChildren() is written as the text "null" (the Map header, L062-9).
function kids(...xs) {
  return xs.flat(Infinity).filter((x) => x !== null && x !== undefined && x !== false && x !== "");
}

/// The page's top bar: icon, title (the menu word), a one-line subtitle, an optional crumb link
/// above the title, and actions on the right. Replaces #top; never draws an empty child.
function pageHeader({ icon: ic, title, sub, crumb, actions } = {}) {
  const top = document.getElementById("top");
  const head = h("div", { class: "ph-title" },
    crumb ? h("a", { class: "lk s", href: crumb.href }, crumb.label) : null,
    h("h1", { tabindex: "-1" }, title || ""),
    sub ? h("span", { class: "s muted" }, sub) : null);
  const right = kids(actions);
  top.replaceChildren(...kids(ic ? icon(ic) : null, head, right.length ? h("span", { class: "right btns tight" }, right) : null));
  return top;
}

let moreSeq = 0;
/// "More options": a button with aria-expanded, closed unless `open`. `changed` (a number) says
/// how many options differ from the default, so a closed section still shows that something is set.
function moreOptions({ label = "More options", changed = 0, open = false } = {}, ...content) {
  const id = `more-${++moreSeq}`;
  const panel = h("div", { class: "more-body", id }, kids(content));
  panel.hidden = !open;
  const btn = h("button", { type: "button", class: "more-btn", "aria-expanded": String(!!open), "aria-controls": id },
    icon("chev", "sm"), label, changed ? h("span", { class: "s muted" }, ` · ${changed} changed`) : null);
  btn.addEventListener("click", () => {
    const now = btn.getAttribute("aria-expanded") !== "true";
    btn.setAttribute("aria-expanded", String(now));
    panel.hidden = !now;
  });
  return h("section", { class: "more" }, btn, panel);
}

/// An in-page confirm instead of the browser's confirm(): says what is lost and what stays; the
/// safe button comes first and has the focus; Esc keeps. With `typed`, the action waits until that
/// word is typed. Resolves true only when the action button is pressed; focus goes back.
function confirmDialog({ title, text, verb, keep = "Keep it", typed, danger = true } = {}) {
  return new Promise((resolve) => {
    const back = document.activeElement;
    const tid = `cd-t-${++moreSeq}`, did = `cd-d-${moreSeq}`;
    const done = (v) => {
      dlg.close();
      dlg.remove();
      if (back && back.isConnected && back.focus) back.focus();
      resolve(v);
    };
    const safe = h("button", { type: "button", class: "btn", onclick: () => done(false) }, keep);
    const go = h("button", { type: "button", class: "btn p" + (danger ? " danger-fill" : ""), onclick: () => done(true) }, verb);
    let word = null;
    if (typed) {
      go.disabled = true;
      word = h("input", { type: "text", "aria-label": `Type ${typed} to confirm`, autocomplete: "off", spellcheck: "false" });
      word.addEventListener("input", () => { go.disabled = word.value.trim() !== typed; });
    }
    const dlg = h("dialog", { class: "cdlg", role: "alertdialog", "aria-modal": "true", "aria-labelledby": tid, "aria-describedby": did },
      h("h2", { id: tid }, title),
      h("p", { id: did }, text),
      typed ? h("label", { class: "fld" }, h("span", { class: "l" }, `Type ${typed} to confirm`), word) : null,
      h("div", { class: "btns tight" }, safe, go));
    dlg.addEventListener("cancel", (e) => { e.preventDefault(); done(false); });
    document.body.append(dlg);
    dlg.showModal();
    safe.focus();
  });
}

/// Nothing here yet: an icon, one bold line, one sentence, one main button.
function empty({ icon: ic, title, text, action } = {}) {
  return h("div", { class: "empty" }, ...kids(
    ic ? icon(ic, "big") : null,
    title ? h("b", {}, title) : null,
    text ? h("span", {}, text) : null,
    action ? h("div", { class: "btns" }, h("button", { type: "button", class: "btn p", onclick: action.onclick }, action.icon ? icon(action.icon, "sm") : null, action.label)) : null));
}

const NOTICE = {
  ok: ["Done", "status"],
  info: ["Note", "status"],
  warn: ["Warning", "alert"],
  error: ["Error", "alert"],
};
/// A message in the page: ok, info, warn or error, always said in words (never colour alone).
function notice(kind, ...text) {
  const [word, role] = NOTICE[kind] || NOTICE.info;
  return h("div", { class: `notice ${NOTICE[kind] ? kind : "info"}`, role }, h("b", {}, `${word}: `), ...kids(text));
}

window.UI = { kids, pageHeader, moreOptions, confirmDialog, empty, notice };
})();
