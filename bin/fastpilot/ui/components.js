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

/// v0.7.1 FC4b: no hidden "More options" anywhere (the user, 24 Nov): what was behind one is a
/// visible section with its own heading.
function section(label, ...content) {
  return h("section", { class: "more" }, h("h3", { class: "more-h" }, label), h("div", { class: "more-body" }, kids(content)));
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

/// E2: ask for one password, once, in a masked field. It goes to this Mac's key store for the
/// current app and is never shown or sent back to the page. `onSaved()` runs after it is kept.
function passwordAsk({ name, label, onSaved, plain = false } = {}) {
  return new Promise((resolve) => {
    const back = document.activeElement;
    const tid = `pw-t-${++moreSeq}`;
    // `plain`: a value that is not secret (a test account's user name) is shown as it is typed.
    const input = h("input", { type: plain ? "text" : "password", autocomplete: "off", spellcheck: "false", "aria-label": label || `Value of ${name}`, "aria-describedby": `${tid}-d` });
    const msg = h("p", { class: "err-line", role: "alert", hidden: true });
    const done = (v) => { dlg.close(); dlg.remove(); if (back && back.isConnected && back.focus) back.focus(); resolve(v); };
    const save = h("button", { type: "submit", class: "btn p" }, plain ? "Save user name" : "Save password");
    const form = h("form", { class: "form" },
      h("h2", { id: tid }, label || `Set ${name}`),
      h("p", { id: `${tid}-d`, class: "s muted" }, "It is saved in this Mac's Keychain for the current app and never shown again. fastpilot types it only on this app's own website."),
      input, msg,
      h("div", { class: "btns tight" }, h("button", { type: "button", class: "btn", onclick: () => done(false) }, "Cancel"), save));
    const dlg = h("dialog", { class: "cdlg", "aria-labelledby": tid }, form);
    form.addEventListener("submit", async (e) => {
      e.preventDefault();
      save.disabled = true;
      try {
        await api("/api/secrets", { action: "set", name, value: input.value });
        input.value = "";
        done(true);
        if (onSaved) onSaved();
      } catch (x) { msg.textContent = x.message; msg.hidden = false; save.disabled = false; input.focus(); }
    });
    dlg.addEventListener("cancel", (e) => { e.preventDefault(); done(false); });
    document.body.append(dlg);
    dlg.showModal();
    input.focus();
  });
}

window.UI = { kids, pageHeader, section, confirmDialog, empty, notice, passwordAsk };
})();
