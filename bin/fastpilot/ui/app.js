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
let savedAt = null;
let doctorCache = null;
const get = (key) => S.settings.find((s) => s.key === key);

async function loadSettings() {
  S = await api("/api/settings");
}
async function save(key, value) {
  try {
    S = await api("/api/settings", { key, value });
    savedAt = new Date();
    render();
  } catch (e) {
    toast("Tidak tersimpan: " + e.message);
    render();
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
  const add = () => {
    const v = input.value.trim();
    if (!v) return;
    if (v.includes(",")) return toast("Satu item per kali, tanpa koma.");
    if (items.includes(v)) return toast(`"${v}" sudah ada.`);
    save(key, [...items, v]);
  };
  input.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); add(); } });
  return h("div", { class: "chips" },
    items.map((it) =>
      h("span", { class: "chip" + (opts.red ? " x" : "") }, opts.show ? opts.show(it) : it,
        locked ? null : h("button", { class: "rm", "aria-label": `Hapus ${it}`, onclick: () => save(key, items.filter((x) => x !== it)) }, icon("close", "sm")))),
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
  if (!savedAt) return h("small", {}, "");
  const t = savedAt.toLocaleTimeString("id-ID", { hour: "2-digit", minute: "2-digit" });
  return h("small", {}, `✓ Tersimpan · ${t}`);
}

// ---- pages ----
function settingsNav(cur) {
  const item = (id, label) => h("a", { href: `#/settings/${id}`, class: cur === id ? "on" : "", "aria-current": cur === id ? "page" : false }, label);
  return h("nav", { class: "snav", "aria-label": "Pengaturan" }, item("model", "Model AI"), item("limits", "Batasan & izin"), item("secret", "Secret"));
}

function topSettings() {
  $top().replaceChildren(icon("settings"), "Pengaturan");
}

const MODELS = [
  ["claude-haiku-5-5", "Claude Code · Haiku 5.5"],
  ["claude-sonnet-5-5", "Claude Code · Sonnet 5.5"],
  ["claude-opus-5-5", "Claude Code · Opus 5.5"],
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
        chips("allowed_domains", { placeholder: "toko.com", addLabel: "Tambah domain" }), get("allowed_domains"))),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("risk"), "aksi berisiko"),
      opt("Tombol yang butuh konfirmasi", "AI berhenti dan harus menulis confirm: true sebelum menekan. Cocok per kata.",
        chips("confirm_actions", { placeholder: "kata, mis. Hapus", addLabel: "Tambah kata konfirmasi", show: confirmShow }), get("confirm_actions")),
      opt("Tombol yang selalu diblokir", null,
        chips("block_actions", { placeholder: "kata, mis. Bayar", addLabel: "Tambah kata yang diblokir", red: true }), get("block_actions"),
        h("div", { class: "d" }, "Diblokir kalau teks tombol ", h("b", {}, "berisi"), " kata ini (huruf besar/kecil diabaikan), juga dengan confirm: true. Contoh: \"Bayar\" juga memblokir \"Lanjut ke Pembayaran\"."))),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("device"), "device & aplikasi"),
      opt("Boleh install / uninstall aplikasi", "Langkah installApp dan uninstallApp", toggle("allow_app_install", "Boleh install / uninstall aplikasi"), get("allow_app_install")),
      opt("Boleh hapus data aplikasi (reset)", "Langkah clearState dan clearKeychain", toggle("allow_app_reset", "Boleh hapus data aplikasi"), get("allow_app_reset")),
      opt("Boleh pakai HP fisik", "Kalau mati, hanya emulator dan simulator", toggle("allow_physical_devices", "Boleh pakai HP fisik"), get("allow_physical_devices")),
      opt("Boleh ubah setting HP (jaringan, lokasi, izin app, bahasa)", "setNetwork, setLocation, setPermissions, setLocale di Android dan iOS", toggle("allow_device_settings", "Boleh ubah setting HP"), get("allow_device_settings"))),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("lock"), "data & keamanan"),
      opt("Jalankan JavaScript bebas (evalScript)", "Matikan untuk mencegah AI mengubah halaman di luar langkah test", toggle("allow_eval_script", "Jalankan JavaScript bebas"), get("allow_eval_script")),
      opt("Folder yang boleh di-upload", "Untuk langkah uploadFile. Kosong = folder mana saja.",
        chips("upload_dirs", { placeholder: "~/fastpilot/fixtures", addLabel: "Tambah folder" }), get("upload_dirs")),
      opt("Umur login tersimpan", "saveState / loadState menolak login yang lebih tua",
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
        chips("vars", { placeholder: "NAMA_SECRET", addLabel: "Tambah nama secret" }), get("vars")),
      h("div", { class: "opt" }, h("div", {}, h("div", { class: "l" }, "Di mana nilainya?"),
        h("div", { class: "d" }, "Isi nilainya di konfigurasi MCP, misalnya ", code("claude mcp add fastpilot -e DEMO_PASS=… -- fastpilot mcp"), ", atau sebagai environment saat menjalankan fastpilot.")))));
}

function pageSettings(sub) {
  topSettings();
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
  const top = [icon("device"), "Device"];
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

// ---- router ----
function render() {
  if (!S) return;
  const hash = location.hash || "#/settings/limits";
  const [, page, sub] = hash.split("/");
  document.querySelectorAll(".rail a").forEach((a) => {
    const on = a.dataset.nav === page;
    a.classList.toggle("on", on);
    if (on) a.setAttribute("aria-current", "page"); else a.removeAttribute("aria-current");
  });
  if (page === "device") {
    if (!doctorCache && !doctorBusy) loadDoctor(false);
    pageDevice();
  } else {
    pageSettings(sub);
  }
}

window.addEventListener("hashchange", () => { render(); document.getElementById("main").focus({ preventScroll: true }); });
window.addEventListener("DOMContentLoaded", async () => {
  try {
    await loadSettings();
    render();
  } catch (e) {
    $top().replaceChildren("fastpilot");
    $body().replaceChildren(h("div", { class: "err", role: "alert" }, "Tidak bisa memuat pengaturan: " + e.message));
  }
});
