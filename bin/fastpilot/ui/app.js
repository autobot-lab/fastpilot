// fastpilot ui: settings (Model AI, Batasan & izin, Secret) and Device. Plain DOM, no framework.
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

function toast(msg) {
  document.querySelectorAll(".toast").forEach((t) => t.remove());
  const t = h("div", { class: "toast", role: "alert" }, msg);
  document.body.append(t);
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.remove(), 7000);
}

async function api(path, body) {
  const opts = body === undefined ? {} : { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) };
  const r = await fetch(path, opts);
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
    toast("Tidak tersimpan: " + e.message);
    render();
    const again = document.querySelector(`input[data-key="${key}"]`);
    if (again) again.focus();
  }
}

// ---- controls ----
function envNote(s) {
  return s.source === "env" ? h("div", { class: "env" }, `Diatur lewat env ${s.env} (menang atas file); ubah di konfigurasi MCP.`) : null;
}
function opt(label, desc, control, s, extra) {
  return h("div", { class: "opt" }, h("div", {}, h("div", { class: "l" }, label), desc ? h("div", { class: "d" }, desc) : null, extra || null, s ? envNote(s) : null), control);
}
function toggle(key, label) {
  const s = get(key);
  const on = s.value === null ? true : !!s.value;
  return h("button", {
    class: "tg", role: "switch", "aria-checked": String(on), "aria-label": label, disabled: s.source === "env",
    onclick: () => save(key, !on),
  });
}
function chips(key, opts) {
  const s = get(key);
  const items = Array.isArray(s.value) ? s.value : [];
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
    if (v.includes(",")) return toast("Satu item per kali, tanpa koma.");
    if (items.some((x) => x.toLowerCase() === v.toLowerCase())) return toast(`"${v}" sudah ada di daftar.`);
    save(key, [...items, v], v);
  };
  input.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); add(); } });
  // A message about the last attempt goes away as soon as the user changes the text.
  input.addEventListener("input", () => { delete typed[key]; clearToast(); });
  return h("div", { class: "chips" },
    empty,
    items.map((it) =>
      h("span", { class: "chip" + (opts.red ? " x" : "") }, opts.show ? opts.show(it) : it,
        locked ? null : h("button", { class: "rm", "aria-label": `Hapus ${it}`, onclick: () => save(key, items.filter((x) => x !== it)) }, icon("close", "sm")))),
    preset,
    locked ? null : h("span", { class: "add" }, input, h("button", { onclick: add, "aria-label": opts.addLabel }, icon("plus", "sm"), "Tambah")));
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
  const t = savedAt.at.toLocaleTimeString("id-ID", { hour: "2-digit", minute: "2-digit" });
  return h("small", {}, `✓ Tersimpan · ${t}`);
}

// ---- pages ----
function settingsNav(cur) {
  const item = (id, label) => h("a", { href: `#/settings/${id}`, class: cur === id ? "on" : "", "aria-current": cur === id ? "page" : false }, label);
  return h("nav", { class: "snav", "aria-label": "Pengaturan" }, item("model", "Model AI"), item("limits", "Batasan & izin"), item("secret", "Secret"));
}

function topSettings() {
  $top().replaceChildren(icon("settings"), h("h1", { tabindex: "-1" }, "Pengaturan"));
}

const MODELS = [
  ["claude-haiku-5-5", "Claude Code · Haiku 5.5"],
  ["claude-sonnet-5-5", "Claude Code · Sonnet 5.5"],
  ["claude-opus-5-5", "Claude Code · Opus 5.5"],
  ["claude-fable-5-2", "Claude Code · Fable 5.2"],
];

function pageModel() {
  const run = doctorCache && doctorCache.checks.find((c) => c.area === "run");
  const status = !doctorCache ? h("span", { class: "tag" }, "Mengecek…")
    : run && run.status === "ok" ? h("span", { class: "tag" }, "✓ Terpasang")
    : h("span", { class: "tag warn" }, "Belum terpasang");
  const replay = [["", `Bawaan · ${MODELS.find(([v]) => v === S.defaults.model_replay)?.[1] || S.defaults.model_replay}`], ...MODELS];
  const explore = [["", `Bawaan · ${MODELS.find(([v]) => v === S.defaults.model_explore)?.[1] || S.defaults.model_explore}`], ...MODELS];
  return h("section", { class: "panel" },
    h("h2", {}, "Model AI", savedNote()),
    h("p", { class: "sub" }, "Model hanya dipakai saat fastpilot menjalankan test sendiri (", code("fastpilot run"), "). Dari Claude Code atau Cursor, model dipilih di aplikasi itu."),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("model"), "model ai · siapa menjalankan apa"),
      opt("Replay (cepat & murah)", "Menjalankan ulang langkah yang sudah tercatat", select("model_replay", replay, "Model replay"), get("model_replay")),
      opt("Eksplorasi / perbaikan", "Dipakai saat replay gagal atau test baru", select("model_explore", explore, "Model eksplorasi"), get("model_explore")),
      opt("Claude Code", run ? `${run.detail}` : "Memakai login Claude Code milikmu, tanpa API key", status)));
}

function pageLimits() {
  const confirmShow = (w) => (w === "default" ? "default (hapus, bayar, beli, …)" : w);
  return h("section", { class: "panel" },
    h("h2", {}, "Batasan & izin", savedNote()),
    h("p", { class: "sub" }, "Aturan ini berlaku untuk semua test: dari Claude Code, Cursor, CLI, maupun ", code("fastpilot run"), "."),
    h("div", { class: "banner" }, icon("info"), h("span", {}, "Perubahan berlaku mulai run berikutnya. Sesi yang sedang berjalan tetap memakai aturan saat ia dimulai. AI lewat chat hanya bisa memperketat aturan, tidak melonggarkan. Disimpan di ", code(S.path), ".")),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("globe"), "domain yang boleh dibuka"),
      opt("Hanya izinkan domain ini", "Kosong = semua domain boleh. Link, redirect, dan popup ke domain lain diblokir.",
        chips("allowed_domains", { placeholder: "mis. toko.com", addLabel: "Tambah domain", empty: "Semua domain boleh" }), get("allowed_domains"))),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("risk"), "aksi berisiko"),
      opt("Tombol yang butuh konfirmasi", "AI berhenti dan harus menyatakan sengaja menekan (confirm: true). Cocok per kata. Bawaan: kosong.",
        chips("confirm_actions", { placeholder: "mis. Hapus", addLabel: "Tambah kata konfirmasi", show: confirmShow, empty: "Belum ada", preset: ["default", "Daftar bawaan (hapus, bayar, beli, …)"] }), get("confirm_actions")),
      opt("Tombol yang selalu diblokir", null,
        chips("block_actions", { placeholder: "mis. Bayar", addLabel: "Tambah kata yang diblokir", red: true, empty: "Belum ada" }), get("block_actions"),
        h("div", { class: "d" }, "Diblokir kalau teks tombol ", h("b", {}, "berisi"), " kata ini (huruf besar/kecil diabaikan), juga dengan confirm: true. Contoh: \"Bayar\" juga memblokir \"Lanjut ke Pembayaran\"."))),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("device"), "device & aplikasi"),
      opt("Boleh install / uninstall aplikasi", "Memasang atau menghapus aplikasi di HP, emulator, atau simulator", toggle("allow_app_install", "Boleh install / uninstall aplikasi"), get("allow_app_install")),
      opt("Boleh hapus data aplikasi (reset)", "Mengosongkan data dan login aplikasi, untuk test yang mulai dari awal", toggle("allow_app_reset", "Boleh hapus data aplikasi"), get("allow_app_reset")),
      opt("Boleh pakai HP fisik", "Kalau mati, hanya emulator dan simulator", toggle("allow_physical_devices", "Boleh pakai HP fisik"), get("allow_physical_devices")),
      opt("Boleh ubah setting HP (jaringan, lokasi, izin app, bahasa)", "Contoh: mode offline, GPS palsu, izin kamera, bahasa HP", toggle("allow_device_settings", "Boleh ubah setting HP"), get("allow_device_settings"))),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("lock"), "data & keamanan"),
      opt("Jalankan JavaScript bebas di halaman", "Matikan untuk mencegah AI mengubah halaman di luar langkah test", toggle("allow_eval_script", "Jalankan JavaScript bebas"), get("allow_eval_script")),
      opt("Folder yang boleh di-upload", "File yang diunggah test harus berasal dari folder ini.",
        chips("upload_dirs", { placeholder: "mis. ~/fixtures", addLabel: "Tambah folder", empty: "Folder mana saja" }), get("upload_dirs")),
      opt("Umur login tersimpan", "Login tersimpan yang lebih tua dari ini tidak dipakai lagi",
        select("state_expire_days", [["", "30 hari (bawaan)"], ["1", "1 hari"], ["7", "7 hari"], ["14", "14 hari"], ["90", "90 hari"]], "Umur login tersimpan"), get("state_expire_days")),
      opt("Sembunyikan secret di semua output", "Nilai secret tampil sebagai *** di output, laporan, dan rekaman", h("span", { class: "fixed" }, "Selalu aktif"))),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("gauge"), "batas pemakaian · fastpilot run"),
      opt("Maksimal langkah AI per test", "Test dihentikan dan ditandai gagal kalau lewat",
        select("max_steps", [["", "30 langkah (bawaan)"], ["10", "10 langkah"], ["20", "20 langkah"], ["40", "40 langkah"], ["60", "60 langkah"], ["100", "100 langkah"]], "Maksimal langkah"), get("max_steps")),
      opt("Maksimal durasi per test", "Test dihentikan dan ditandai gagal kalau lewat",
        select("test_timeout", [["", "30 menit (bawaan)"], ["5", "5 menit"], ["10", "10 menit"], ["15", "15 menit"], ["60", "60 menit"]], "Maksimal durasi"), get("test_timeout"))));
}

function pageSecret() {
  return h("section", { class: "panel" },
    h("h2", {}, "Secret", savedNote()),
    h("p", { class: "sub" }, "Variabel yang boleh dibaca flow sebagai ", code("${NAMA}"), ". Nilainya selalu tampil *** di output."),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("key"), "secret"),
      opt("Nama secret", "Hanya nama yang disimpan di sini; nilainya tidak pernah dibaca atau ditampilkan halaman ini.",
        chips("vars", { placeholder: "mis. DEMO_PASS", addLabel: "Tambah nama secret", empty: "Belum ada" }), get("vars")),
      h("div", { class: "opt" }, h("div", {}, h("div", { class: "l" }, "Di mana nilainya?"),
        h("div", { class: "d" }, "Isi nilainya di konfigurasi MCP, misalnya ", code("claude mcp add fastpilot -e DEMO_PASS=… -- fastpilot mcp"), ", atau sebagai environment saat menjalankan fastpilot.")))));
}

// The settings file does not parse: say what and where, and offer nothing to change (the server
// refuses to save until it is fixed; fastpilot itself stops with the same error).
function pageBroken() {
  const reload = async () => {
    try { await loadSettings(); } catch (e) { toast("Gagal memuat ulang: " + e.message); }
    render();
    // Keyboard focus stays on the page (the button it was on is gone): the page title.
    document.querySelector(".top h1")?.focus();
  };
  return h("section", { class: "panel" },
    h("h2", {}, "Pengaturan"),
    h("div", { class: "err big-err", role: "alert" },
      h("b", {}, "File pengaturan rusak. "),
      "Test tidak akan jalan, dan tidak ada perubahan yang bisa disimpan, sampai file ini diperbaiki atau dihapus.",
      h("div", { class: "path" }, code(S.path)),
      h("pre", { class: "errtext" }, S.error)),
    h("div", { class: "btns" }, h("button", { class: "btn p", onclick: reload }, icon("refresh", "sm"), "Muat ulang setelah diperbaiki")),
    h("p", { class: "muted" }, "Sama dengan pesan ", code("fastpilot config list"), ". Menghapus file mengembalikan semua ke bawaan."));
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
    top.push(h("span", { class: "pill" + (bad ? " grey" : "") }, `● ${ok} siap · ${bad} perlu perhatian`));
  }
  $top().replaceChildren(...top);
  const header = h("div", { class: "gh" }, icon("device"), "device · hasil doctor",
    h("button", { class: "btn sm", onclick: () => loadDoctor(), disabled: doctorBusy }, icon("refresh", "sm"), doctorBusy ? "Mengecek…" : "Cek ulang (doctor)"));
  let content;
  if (doctorBusy || !doctorCache) {
    content = h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Mengecek" }), h("b", {}, "Mengecek device…"),
      h("span", {}, "Chrome, adb, Xcode, dan Claude Code. Biasanya beberapa detik, paling lama sekitar 30 detik."));
  } else if (doctorCache.error) {
    content = h("div", { class: "err", role: "alert" }, "Gagal menjalankan doctor: " + doctorCache.error);
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
            c.setup ? h("div", { class: "s" }, "Bisa diperbaiki otomatis (bertanya dulu): ", code(c.setup)) : null),
          h("span", { class: "tag" + (c.status === "ok" ? "" : " " + c.status) }, c.status === "ok" ? "Siap" : c.status === "warn" ? "Perhatian" : "Gagal"))),
      a === "android" && physOff ? h("div", { class: "row" }, h("div", { class: "s" }, "HP fisik dimatikan di Batasan, jadi hanya emulator yang dipakai. ", h("a", { class: "lk", href: "#/settings/limits" }, "Ubah di Batasan"))) : null,
    ]);
  }
  $body().replaceChildren(h("div", { class: "dev" },
    h("div", { class: "grp" }, header, content),
    h("p", { class: "muted" }, "Sama dengan ", code("fastpilot doctor"), " di terminal. Perbaikan otomatis dijalankan dari terminal dengan ", code("fastpilot setup"), ", yang selalu bertanya dulu."),
    h("p", { class: "muted" }, "Model AI dan Secret ada di Pengaturan: ", h("a", { class: "lk", href: "#/settings/model" }, "Model AI"), " · ", h("a", { class: "lk", href: "#/settings/secret" }, "Secret"), ".")));
}

// ---- live (C7a) ----
const PLATFORM = { web: "chrome · milik fastpilot", android: "android", ios: "ios" };
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
  if (at < 0) return steps.length ? `0/${steps.length} langkah` : "Belum ada langkah";
  return `Langkah ${at + 1}/${steps.length} · ${steps[at].label}`;
}

function sessionStatus(s) {
  if (s.taken_over_ms) return { text: "Diambil alih olehmu · langkah otomatis menunggu", cls: "warn" };
  const f = s.flow;
  if (!f) return { text: "Siap · menunggu langkah", cls: "" };
  if (f.steps.some((x) => x.state === "blocked")) return { text: "Diblokir Batasan", cls: "bad" };
  if (f.steps.some((x) => x.state === "failed")) return { text: "Gagal", cls: "bad" };
  if (!f.finished) return { text: "Sedang berjalan", cls: "run" };
  return { text: `Selesai · ${f.steps.filter((x) => x.state === "done").length}/${f.steps.length} lolos`, cls: "ok" };
}

// Why a guard rail stopped a step, in a few words, from the engine's message.
function blockReason(note) {
  const n = note || "";
  // The engine compares in lower case; show the word as the user wrote it in Batasan.
  const asWritten = (key, w) => {
    const list = (get(key) && Array.isArray(get(key).value)) ? get(key).value : [];
    return list.find((x) => x.toLowerCase() === w.toLowerCase()) || w;
  };
  let m = /contains "([^"]+)" \(block_actions\)/.exec(n);
  if (m) return `teks berisi "${asWritten("block_actions", m[1])}"`;
  m = /risky action \("([^"]+)"/.exec(n);
  if (m) return `butuh confirm: true ("${asWritten("confirm_actions", m[1])}")`;
  if (/allowed_domains|ALLOWED_DOMAINS|not in the allowed/i.test(n)) return "domain di luar daftar";
  m = /\((allow_[a-z_]+) = false\)/.exec(n);
  if (m) return `dimatikan di Batasan (${m[1]})`;
  if (/upload_dirs/.test(n)) return "file di luar folder upload";
  return "aturan Batasan";
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
    if (!shown) throw new Error("tidak ada gambar dari sesi ini");
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
      ? "Chrome milikmu (attach) tidak ditampilkan di sini."
      : `Layar ${s.platform} tidak bisa ditampilkan: proses fastpilot ini versi lama. Langkahnya tetap terlihat di bawah.`));
  }
  // Android/iOS frames are screenshots from the device (the server caps them at 3 per second).
  const fps = big ? (s.platform === "web" ? 8 : 3) : (s.platform === "web" ? 2 : 1), w = big ? 1280 : 480;
  const img = h("img", { class: "screen", alt: `Layar sesi ${s.id}`, draggable: "false" });
  const status = h("div", { class: "screen-state" }, h("div", { class: "spin", role: "status", "aria-label": "Memuat" }), h("span", {}, "Menyambung ke layar…"));
  const box = h("div", { class: "screen-box" }, img, status);
  const start = () => watch(img, `/api/live/frames?pid=${s.pid}&session=${encodeURIComponent(s.id)}&fps=${fps}&w=${w}`, () => status.remove(), (why) => {
    status.replaceChildren(h("span", { class: "bad" }, "Layar tidak bisa ditampilkan: " + why),
      h("button", { class: "btn sm", onclick: () => { status.replaceChildren(h("div", { class: "spin" }), h("span", {}, "Menyambung ke layar…")); start(); } }, "Coba lagi"));
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
    ? h("button", { class: "btn sm", "data-fk": `zoom:${key}`, "aria-label": `Perbesar layar sesi ${s.id}`, onclick: (e) => openZoom(s, e.currentTarget) }, icon("expand", "sm"), "Perbesar")
    : null;
  const take = s.controllable
    ? h("button", { class: "btn sm", "data-fk": `take:${key}`, onclick: (e) => toggleTakeover(key, e.currentTarget) })
    : null;
  const more = zoom || take ? h("div", { class: "btns tight" }, zoom, take) : null;
  const el = h("article", { class: "lcard", tabindex: "0", "data-fk": `card:${key}`, "aria-label": `Sesi ${s.id}, ${s.platform}. Enter untuk melihat langkahnya.`,
    onclick: (e) => { if (!e.target.closest("button")) selectSession(key); } },
    h("div", { class: "lh" }, h("span", { class: "pf " + s.platform }, s.platform), h("span", { class: "muted mono" }, PLATFORM[s.platform] || s.platform), h("span", { class: "chip" }, s.id)),
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
  const limits = (i) => h("a", { class: "lk bad", href: "#/settings/limits", "data-fk": `limits:${i}` }, "Lihat Batasan");
  const items = steps.map((x, i) => {
    const n = `${i + 1} · `;
    if (x.state === "running") return h("li", { class: "now" }, h("span", { class: "i" }, icon("play", "sm")), h("span", {}, n, x.label), h("span", { class: "r" }, "sedang berjalan"));
    if (x.state === "done") return h("li", {}, h("span", { class: "i ok", "aria-label": "selesai" }, "✓"), h("span", {}, n, x.label));
    if (x.state === "failed") return h("li", { class: "blk" }, h("span", { class: "i", "aria-label": "gagal" }, "✗"), h("span", {}, n, x.label), h("span", { class: "r" }, x.note || "gagal"));
    if (x.state === "blocked") return h("li", { class: "blk" }, h("span", { class: "i", "aria-label": "diblokir" }, icon("lock", "sm")), h("span", {}, n, x.label),
      h("span", { class: "r" }, `diblokir: ${blockReason(x.note)} · `, limits(i)));
    if (x.state === "skipped") return h("li", { class: "muted" }, h("span", { class: "i" }, "–"), h("span", {}, n, x.label, " · dilewati"));
    return h("li", { class: x.will_block ? "warnline" : "" }, h("span", { class: "i" }, x.will_block ? icon("lock", "sm") : "○"), h("span", {}, n, x.label),
      x.will_block ? h("span", { class: "r" }, x.will_block, " · ", limits(i)) : null);
  });
  return h("div", { class: "grp steps-grp" },
    h("h2", { class: "gh" }, icon("tests"), `riwayat langkah · ${s.id} (${s.platform} · sesi yang dipilih)`),
    items.length ? h("ul", { class: "steps", "aria-live": "polite" }, items) : h("div", { class: "empty small" }, "Sesi ini belum menjalankan langkah."));
}

function liveEmpty() {
  return h("div", { class: "grp" }, h("div", { class: "gh" }, "live · kosong"),
    h("div", { class: "empty" }, icon("live", "big"), h("b", {}, "Belum ada test yang berjalan"),
      h("span", {}, "Sesi dari Claude Code (MCP), CLI, atau ", code("fastpilot run"), " muncul di sini secara live, tanpa mengganggu Chrome atau HP-mu."),
      h("span", {}, "Coba: ", code("fastpilot open https://example.com"))));
}

// ---- Ambil alih (C7b) ----
// The user steers a session from here; its automatic steps wait until Kembalikan.
function liveSession(key) {
  return (lastLive.sessions || []).find((x) => `${x.pid}:${x.id}` === key);
}

function setTakeLabel(btn, s) {
  const taken = !!s.taken_over_ms;
  const label = taken ? "Kembalikan" : "Ambil alih";
  if (btn.dataset.label !== label) {
    btn.dataset.label = label;
    btn.replaceChildren(icon(taken ? "play" : "cursor", "sm"), label);
    btn.classList.toggle("p", taken);
    btn.setAttribute("aria-label", taken ? `Kembalikan sesi ${s.id} ke otomatis` : `Ambil alih sesi ${s.id}: langkah otomatis dijeda`);
  }
}

async function control(s, op, extra) {
  try {
    await api("/api/live/control", Object.assign({ pid: s.pid, session: s.id, op }, extra || {}));
    return true;
  } catch (e) {
    toast((op === "takeover" ? "Tidak bisa mengambil alih: " : op === "release" ? "Tidak bisa mengembalikan: " : "Input tidak terkirim: ") + e.message);
    return false;
  }
}

async function refreshLive() {
  try { renderLive(await api("/api/live")); } catch (_) { /* the next poll shows it */ }
}

async function toggleTakeover(key, opener) {
  const s = liveSession(key);
  if (!s) return;
  if (s.taken_over_ms) {
    if (await control(s, "release")) await refreshLive();
    return;
  }
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
    if (!s || !s.taken_over_ms) return;
    e.preventDefault();
    down = fraction(img, e);
  });
  img.addEventListener("pointerup", async (e) => {
    const s = liveSession(key);
    const from = down;
    down = null;
    if (!s || !s.taken_over_ms || !from) return;
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
  const text = h("input", { type: "text", class: "steer-text", maxlength: "500", placeholder: "Ketik ke kolom yang aktif", "aria-label": "Teks untuk diketik di device" });
  const sendText = async () => {
    const s2 = liveSession(key);
    if (s2 && text.value && await control(s2, "type", { text: text.value })) text.value = "";
  };
  text.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); sendText(); } });
  const key_ = (k, label) => h("button", { class: "btn sm", onclick: () => { const s2 = liveSession(key); if (s2) control(s2, "key", { key: k }); } }, label);
  const back = s && s.platform !== "web"
    ? h("button", { class: "btn sm", onclick: () => { const s2 = liveSession(key); if (s2) control(s2, "back"); } }, "Kembali")
    : null;
  const on = h("div", { class: "steer-on" },
    h("p", { class: "muted small" }, "Klik layar untuk tap, seret untuk swipe. Langkah otomatis menunggu sampai kamu menekan Kembalikan."),
    h("div", { class: "steer-row" }, text, h("button", { class: "btn sm", onclick: sendText }, "Ketik"), key_("Enter", "Enter"), key_("Backspace", "Hapus"), back));
  const take = h("button", { class: "btn sm", "data-fk": `take-zoom:${key}`, onclick: (e) => toggleTakeover(key, e.currentTarget) });
  const bar = h("div", { class: "steer", "data-key": key }, h("div", { class: "steer-head" }, h("span", { class: "steer-state" }), take), on);
  updateSteer(bar);
  return bar;
}

function updateSteer(bar) {
  const s = liveSession(bar.dataset.key);
  if (!s) return;
  const taken = !!s.taken_over_ms;
  setTakeLabel(bar.querySelector("[data-fk^='take-zoom:']"), s);
  const state = bar.querySelector(".steer-state");
  const t = taken ? "Diambil alih olehmu · langkah otomatis menunggu" : "Otomatis · ambil alih untuk klik dan ketik sendiri";
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
  const dlg = h("dialog", { class: "zoom", "aria-label": `Layar sesi ${s.id}, diperbesar` },
    h("div", { class: "lh" }, h("span", { class: "pf " + s.platform }, s.platform), h("span", { class: "mono" }, s.id),
      h("button", { class: "btn sm right", onclick: () => closeZoom(true) }, icon("close", "sm"), "Tutup")),
    screenEl(s, true),
    s.controllable ? steerBar(`${s.pid}:${s.id}`) : null);
  if (s.controllable) steerable(dlg.querySelector(".screen-box"), `${s.pid}:${s.id}`);
  dlg.querySelectorAll(".steer").forEach(updateSteer);
  dlg.addEventListener("cancel", (e) => { e.preventDefault(); closeZoom(true); });
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
function closeZoom(restore) {
  document.querySelectorAll("dialog.zoom").forEach((d) => { stopWatching(d); if (d.open) d.close(); d.remove(); });
  if (restore && zoomOpener && zoomOpener.isConnected) zoomOpener.focus();
  zoomOpener = null;
}

function livePill(sessions) {
  if (!sessions.length) return h("span", { class: "pill grey" }, "○ Tidak ada sesi berjalan");
  const count = (st) => sessions.filter((s) => s.flow && s.flow.steps.some((x) => x.state === st)).length;
  const running = sessions.filter((s) => s.flow && !s.flow.finished).length;
  const blocked = count("blocked"), failed = count("failed");
  const parts = [`${sessions.length} sesi`, `${running} berjalan`];
  if (blocked) parts.push(`${blocked} diblokir`);
  if (failed) parts.push(`${failed} gagal`);
  return h("span", { class: "pill" + (blocked || failed ? " bad" : "") }, "● " + parts.join(" · "));
}

function renderLive(data) {
  lastLive = data;
  const sessions = data.sessions || [];
  $top().replaceChildren(icon("live"), h("h1", { tabindex: "-1" }, "Live"), livePill(sessions));
  if (!sessions.length) {
    stopWatching();
    liveCards.clear();
    stepsSig = "";
    closeZoom(false);
    $body().replaceChildren(liveEmpty());
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
  if (document.querySelector("dialog.zoom") && zoomOpener && !zoomOpener.isConnected) closeZoom(false);
}

async function pollLive() {
  // An empty hash is the Live page too (the first page after the link).
  if (!(location.hash || "#/live").startsWith("#/live")) return;
  try {
    renderLive(await api("/api/live"));
  } catch (e) {
    $body().replaceChildren(h("div", { class: "err", role: "alert" }, "Tidak bisa membaca sesi: " + e.message));
  }
  liveTimer = setTimeout(pollLive, 1000);
}

// ---- Jadwal (C3) ----
let schedData = null, schedBusy = false, schedForm = null; // schedForm: null | { id?, ... }
const DAYS = ["Senin", "Selasa", "Rabu", "Kamis", "Jumat", "Sabtu", "Minggu"];

async function loadSchedules() {
  schedBusy = true;
  try { schedData = await api("/api/schedules"); } catch (e) { schedData = { error: e.message, schedules: [] }; }
  schedBusy = false;
  if ((location.hash || "").startsWith("#/schedules")) pageSchedules();
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

function ago(ms) {
  const s = Math.round((Date.now() - ms) / 1000);
  if (s < 90) return "baru saja";
  if (s < 3600) return `${Math.round(s / 60)} menit lalu`;
  if (s < 86400) return `${Math.round(s / 3600)} jam lalu`;
  return `${Math.round(s / 86400)} hari lalu`;
}

function lastRun(l) {
  if (!l) return h("span", { class: "muted" }, "Belum jalan");
  if (l.passed === undefined || l.passed === null) return h("span", { class: "run" }, h("div", { class: "spin sm", "aria-hidden": "true" }), " Berjalan · ", ago(l.started_ms));
  return h("div", {},
    h("span", { class: l.passed ? "ok" : "bad" }, l.passed ? "Lolos" : "Gagal", " · ", ago(l.finished_ms || l.started_ms)),
    h("div", { class: "s" }, l.summary || ""),
    l.report ? h("div", { class: "s" }, "Laporan: ", code(l.report)) : null,
    l.notified && l.notified.length ? h("div", { class: "s" }, l.notified.join(" · ")) : null);
}

function notifyChips(x) {
  const n = x.schedule.notify || {};
  const out = [];
  if (n.webhook) out.push(h("span", { class: "chip" }, "Webhook ", x.webhook ? h("span", { class: "muted" }, x.webhook) : null));
  if (n.os) out.push(h("span", { class: "chip" }, "Notifikasi di komputer ini"));
  if (n.github) out.push(h("span", { class: "chip" }, "Issue ", n.github));
  return out.length ? h("div", { class: "chips" }, out) : h("span", { class: "muted" }, "Tidak ada");
}

function schedRow(x) {
  const s = x.schedule;
  const on = h("button", { class: "tg", role: "switch", "aria-checked": String(!!s.enabled), "aria-label": `Jadwal ${s.name} aktif`,
    onclick: () => schedAction({ action: s.enabled ? "disable" : "enable", id: s.id }) });
  const name = s.tests.split("/").filter(Boolean).pop() || s.tests;
  return h("tr", {},
    h("td", {}, h("b", {}, s.name), h("div", { class: "s mono" }, s.id)),
    h("td", {}, code(name), h("div", { class: "s", title: s.tests }, s.tests)),
    h("td", {}, x.when_text),
    h("td", {}, lastRun(x.last)),
    h("td", {}, notifyChips(x)),
    h("td", {}, on),
    h("td", {}, h("div", { class: "acts" },
      h("button", { class: "btn sm", onclick: () => schedAction({ action: "run", id: s.id }, () => toast(`"${s.name}" dijalankan sekarang. Hasilnya muncul di Run terakhir.`)) }, icon("play", "sm"), "Jalankan"),
      h("button", { class: "btn sm", onclick: () => schedAction({ action: "test", id: s.id }, (r) => toast("Tes notifikasi: " + r.sent.join(" · "))) }, "Tes notifikasi"),
      h("button", { class: "btn sm", onclick: () => { schedForm = Object.assign({}, s, { webhook_set: !!x.webhook }); pageSchedules(); } }, "Ubah"),
      h("button", { class: "btn sm danger", onclick: () => { if (confirm(`Hapus jadwal "${s.name}"? Entri di penjadwal sistem ikut dihapus.`)) schedAction({ action: "delete", id: s.id }); } }, "Hapus"))));
}

function schedPanel() {
  const f = schedForm;
  const editing = !!f.id;
  const field = (label, input, hint) => h("label", { class: "fld" }, h("span", { class: "l" }, label), input, hint ? h("span", { class: "s" }, hint) : null);
  const name = h("input", { type: "text", maxlength: "80", value: f.name || "", placeholder: "mis. Checkout hari kerja" });
  const tests = h("input", { type: "text", value: f.tests || (schedData && schedData.tests_hint) || "", placeholder: "/path/lengkap/ke/tests atau file .md", class: "mono" });
  const every = h("select", { class: "sel" },
    [["day", "Setiap hari"], ["weekday", "Hari kerja (Senin–Jumat)"], ["week", "Mingguan"]].map(([v, t]) => h("option", { value: v, selected: (f.when && f.when.every) === v }, t)));
  const day = h("select", { class: "sel" }, DAYS.map((d, i) => h("option", { value: String(i + 1), selected: f.when && f.when.day === i + 1 }, d)));
  const dayWrap = field("Hari", day);
  const syncDay = () => { dayWrap.hidden = every.value !== "week"; };
  every.addEventListener("change", syncDay);
  syncDay();
  const time = h("input", { type: "time", value: (f.when && f.when.time) || "08:00", required: true });
  const n = f.notify || {};
  const wh = h("input", { type: "checkbox", checked: !!n.webhook });
  const whUrl = h("input", { type: "password", autocomplete: "off", placeholder: f.webhook_set ? "tersimpan · isi untuk mengganti" : "https://hooks.slack.com/services/…", class: "mono" });
  const os = h("input", { type: "checkbox", checked: !!n.os });
  const gh = h("input", { type: "checkbox", checked: !!n.github });
  const repo = h("input", { type: "text", value: n.github || "", placeholder: "pemilik/repo", class: "mono" });
  const save = async () => {
    const schedule = {
      id: f.id || "", name: name.value, tests: tests.value, enabled: f.enabled !== false,
      when: { every: every.value, time: time.value, day: every.value === "week" ? Number(day.value) : undefined },
      notify: { webhook: wh.checked, os: os.checked, github: gh.checked ? repo.value.trim() : undefined },
    };
    const body = { action: "save", schedule };
    if (whUrl.value.trim()) body.webhook = whUrl.value.trim();
    const r = await schedAction(body);
    if (r) { schedForm = null; toast(editing ? "Jadwal disimpan." : `Jadwal disimpan dan didaftarkan ke ${schedData.backend}.`); pageSchedules(); }
  };
  return h("section", { class: "grp sched-form", "aria-label": editing ? "Ubah jadwal" : "Jadwal baru" },
    h("h2", { class: "gh" }, icon("clock"), editing ? `ubah jadwal · ${f.id}` : "jadwal baru"),
    h("div", { class: "form" },
      field("Nama", name),
      field("Test", tests, "Folder berisi test (*.md) atau satu file .md, path lengkap. Dijalankan dengan fastpilot run."),
      h("div", { class: "row2" }, field("Kapan", every), dayWrap, field("Jam (waktu komputer ini)", time)),
      h("fieldset", { class: "fld" }, h("legend", { class: "l" }, "Kalau gagal (bisa lebih dari satu)"),
        h("label", { class: "ck" }, wh, " Webhook (kompatibel Slack)"), whUrl,
        h("span", { class: "s" }, "URL webhook disimpan sebagai rahasia (~/.fastpilot/secrets, hanya pemilik) dan tidak ditampilkan lagi."),
        h("label", { class: "ck" }, os, " Notifikasi di komputer ini"),
        h("label", { class: "ck" }, gh, " Issue GitHub di repo"), repo,
        h("span", { class: "s" }, "Token dari GITHUB_TOKEN / GH_TOKEN, atau Keychain item ", code("fastpilot-github"), ".")),
      h("div", { class: "btns" },
        h("button", { class: "btn", onclick: () => { schedForm = null; clearToast(); pageSchedules(); } }, "Batal"),
        h("button", { class: "btn p", onclick: save }, "Simpan jadwal"))));
}

function pageSchedules() {
  $top().replaceChildren(icon("clock"), h("h1", { tabindex: "-1" }, "Jadwal"),
    h("button", { class: "btn sm p right", onclick: () => { schedForm = {}; pageSchedules(); document.querySelector(".sched-form input")?.focus(); } }, icon("plus", "sm"), "Jadwal baru"));
  if (!schedData) {
    $body().replaceChildren(h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Memuat" }), h("b", {}, "Membaca jadwal…")));
    if (!schedBusy) loadSchedules();
    return;
  }
  const list = schedData.schedules || [];
  const where = { launchd: "launchd (LaunchAgent) di Mac ini", systemd: "systemd user timer", cron: "crontab" }[schedData.backend] || schedData.backend;
  const table = list.length
    ? h("div", { class: "grp" }, h("div", { class: "gh" }, icon("clock"), `jadwal · ${list.length} · dijalankan oleh ${where}`),
      h("div", { class: "tbl-wrap" }, h("table", { class: "tbl" },
        h("thead", {}, h("tr", {}, ["Nama", "Test", "Kapan", "Run terakhir", "Kalau gagal", "Aktif", ""].map((t) => h("th", {}, t)))),
        h("tbody", {}, list.map(schedRow)))))
    : h("div", { class: "grp" }, h("div", { class: "gh" }, "jadwal · kosong"),
      h("div", { class: "empty" }, icon("clock", "big"), h("b", {}, "Belum ada jadwal"),
        h("span", {}, "Jalankan test otomatis tiap hari, hari kerja, atau mingguan. Jadwal didaftarkan ke ", where, " hanya saat kamu menyimpannya."),
        h("button", { class: "btn p", onclick: () => { schedForm = {}; pageSchedules(); } }, icon("plus", "sm"), "Jadwal baru")));
  $body().replaceChildren(...(schedData.error ? [h("div", { class: "err", role: "alert" }, "Tidak bisa membaca jadwal: " + schedData.error)] : []), ...(schedForm ? [schedPanel()] : []), table);
}

// ---- router ----
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
      $body().replaceChildren(h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Memuat" }), h("b", {}, "Membaca sesi…")));
      pollLive();
    }
  } else if (page === "schedules") {
    if (!schedData) loadSchedules(); else pageSchedules();
  } else if (page === "device") {
    if (!doctorCache && !doctorBusy) loadDoctor(false);
    pageDevice();
  } else {
    pageSettings(sub);
  }
}

window.addEventListener("hashchange", async () => {
  // Settings are read again on the way in: the file may have changed (or broken) meanwhile.
  if (location.hash.startsWith("#/settings")) {
    try { await loadSettings(); } catch (e) { toast("Gagal memuat pengaturan: " + e.message); }
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
    $body().replaceChildren(h("div", { class: "err", role: "alert" }, "Tidak bisa memuat pengaturan: " + e.message));
  }
});
