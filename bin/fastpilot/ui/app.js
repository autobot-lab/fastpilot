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
const get = (key) => (S && S.settings ? S.settings.find((s) => s.key === key) : undefined);

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

// The settings file does not parse: say what and where, and offer nothing to change (the server
// refuses to save until it is fixed; fastpilot itself stops with the same error).
function pageBroken() {
  const reload = async () => {
    try { await loadSettings(); } catch (e) { toast("Gagal memuat ulang: " + e.message); }
    render();
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

// ---- live (C7a) ----
const PLATFORM = { web: "chrome · milik fastpilot", android: "android", ios: "ios" };
let liveTimer = null;
let liveSel = null; // "pid:id"
let liveBig = null; // "pid:id" shown large
const liveCards = new Map(); // key -> { el, img, text }

function stopLive() {
  clearTimeout(liveTimer);
  liveTimer = null;
  // Ending the streams ends the screencasts on the server.
  stopWatching();
  liveCards.clear();
  document.getElementById("live-root")?.remove();
}

function stepLine(steps) {
  const pick = (st) => steps.findIndex((s) => s.state === st);
  const running = pick("running"), stopped = steps.findIndex((s) => s.state === "blocked" || s.state === "failed");
  const at = running >= 0 ? running : stopped >= 0 ? stopped : steps.map((s) => s.state).lastIndexOf("done");
  if (at < 0) return steps.length ? `0/${steps.length} langkah` : "Belum ada langkah";
  return `Langkah ${at + 1}/${steps.length} · ${steps[at].label}`;
}

function sessionStatus(s) {
  const f = s.flow;
  if (!f) return { text: "Siap · menunggu langkah", cls: "" };
  if (f.steps.some((x) => x.state === "blocked")) return { text: "Diblokir Batasan", cls: "bad" };
  if (f.steps.some((x) => x.state === "failed")) return { text: "Gagal", cls: "bad" };
  if (!f.finished) return { text: "Sedang berjalan", cls: "run" };
  return { text: `Selesai · ${f.steps.filter((x) => x.state === "done").length}/${f.steps.length} lolos`, cls: "ok" };
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
        const h = find(buf, sep, 0);
        if (h < 0) break;
        const head = new TextDecoder().decode(buf.slice(0, h));
        const m = /Content-Length:\s*(\d+)/i.exec(head);
        if (!m) { buf = buf.slice(h + 4); continue; }
        const len = Number(m[1]);
        if (buf.length < h + 4 + len) break;
        const jpeg = buf.slice(h + 4, h + 4 + len);
        buf = buf.slice(h + 4 + len);
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
  const key = `${s.pid}:${s.id}`;
  if (!s.watchable) {
    return h("div", { class: "screen-box none" }, h("span", {}, s.platform === "web"
      ? "Chrome milikmu (attach) tidak ditampilkan di sini."
      : `Layar ${s.platform} langsung menyusul. Langkahnya tetap terlihat di bawah.`));
  }
  const fps = big ? 8 : 2, w = big ? 1280 : 480;
  const img = h("img", { class: "screen", alt: `Layar sesi ${s.id}` });
  const status = h("div", { class: "screen-state" }, h("div", { class: "spin", role: "status", "aria-label": "Memuat" }), h("span", {}, "Menyambung ke layar…"));
  const box = h("div", { class: "screen-box" }, img, status);
  watch(img, `/api/live/frames?pid=${s.pid}&session=${encodeURIComponent(s.id)}&fps=${fps}&w=${w}`, () => status.remove(), (why) => {
    status.replaceChildren(h("span", { class: "bad" }, "Layar tidak bisa ditampilkan: " + why),
      h("button", { class: "btn sm", onclick: () => { const c = liveCards.get(key); if (c) { stopWatching(c.el); c.el.remove(); } liveCards.delete(key); renderLive(lastLive); } }, "Coba lagi"));
    if (!box.contains(status)) box.append(status);
  });
  return box;
}

let lastLive = { sessions: [] };

function card(s) {
  const key = `${s.pid}:${s.id}`;
  const text = h("div", { class: "card-text" });
  const el = h("article", { class: "lcard", tabindex: "0", "aria-label": `Sesi ${s.id}`,
    onclick: (e) => { if (e.target.closest("button")) return; liveSel = key; renderLive(lastLive); } },
    h("div", { class: "lh" }, h("span", { class: "pf " + s.platform }, s.platform), h("span", { class: "muted mono" }, PLATFORM[s.platform] || s.platform), h("span", { class: "chip" }, s.id)),
    screenEl(s, false), text);
  el.addEventListener("keydown", (e) => { if (e.key === "Enter") { liveSel = key; renderLive(lastLive); } });
  return { el, text };
}

function fillCard(c, s) {
  const st = sessionStatus(s);
  const steps = s.flow ? s.flow.steps : [];
  c.text.replaceChildren(
    h("div", {}, h("div", { class: "l" }, stepLine(steps)), h("div", { class: "d " + st.cls }, st.text, s.kind ? ` · ${s.kind === "mcp" ? "Claude Code / MCP" : s.kind === "daemon" ? "CLI" : "fastpilot run"}` : "")),
    s.watchable ? h("button", { class: "btn sm", onclick: () => { liveBig = `${s.pid}:${s.id}`; renderLive(lastLive); } }, icon("expand", "sm"), "Perbesar") : null);
}

function stepsPanel(s) {
  const steps = s.flow ? s.flow.steps : [];
  const items = steps.map((x, i) => {
    const n = `${i + 1} · `;
    if (x.state === "running") return h("li", { class: "now" }, h("span", { class: "i" }, icon("play", "sm")), h("span", {}, n, x.label), h("span", { class: "r" }, "sedang berjalan"));
    if (x.state === "done") return h("li", {}, h("span", { class: "i ok" }, "✓"), h("span", {}, n, x.label));
    if (x.state === "failed") return h("li", { class: "blk" }, h("span", { class: "i" }, "✗"), h("span", {}, n, x.label), h("span", { class: "r" }, x.note || "gagal"));
    if (x.state === "blocked") return h("li", { class: "blk" }, h("span", { class: "i" }, icon("lock", "sm")), h("span", {}, n, x.label),
      h("span", { class: "r" }, "diblokir · ", h("a", { class: "lk bad", href: "#/settings/limits" }, "Lihat Batasan")));
    if (x.state === "skipped") return h("li", { class: "muted" }, h("span", { class: "i" }, "–"), h("span", {}, n, x.label, " · dilewati"));
    return h("li", { class: x.will_block ? "warnline" : "" }, h("span", { class: "i" }, x.will_block ? icon("lock", "sm") : "○"), h("span", {}, n, x.label),
      x.will_block ? h("span", { class: "r" }, x.will_block, " · ", h("a", { class: "lk bad", href: "#/settings/limits" }, "Lihat Batasan")) : null);
  });
  return h("div", { class: "grp steps-grp" },
    h("div", { class: "gh" }, icon("tests"), `riwayat langkah · ${s.id} (${s.platform} · sesi yang dipilih)`),
    items.length ? h("ul", { class: "steps" }, items) : h("div", { class: "empty small" }, "Sesi ini belum menjalankan langkah."));
}

function liveEmpty() {
  return h("div", { class: "grp" }, h("div", { class: "gh" }, "live · kosong"),
    h("div", { class: "empty" }, icon("live", "big"), h("b", {}, "Belum ada test yang berjalan"),
      h("span", {}, "Sesi dari Claude Code (MCP), CLI, atau ", code("fastpilot run"), " muncul di sini secara live, tanpa mengganggu Chrome atau HP-mu."),
      h("span", {}, "Coba: ", code("fastpilot open https://example.com"))));
}

function renderLive(data) {
  lastLive = data;
  const sessions = data.sessions || [];
  const running = sessions.filter((s) => s.flow && !s.flow.finished).length;
  $top().replaceChildren(icon("live"), "Live",
    sessions.length ? h("span", { class: "pill" }, `● ${sessions.length} sesi · ${running} berjalan · headless`) : h("span", { class: "pill grey" }, "○ Tidak ada sesi berjalan"));
  if (!sessions.length) {
    stopWatching();
    liveCards.clear();
    $body().replaceChildren(liveEmpty());
    return;
  }
  const keys = new Set(sessions.map((s) => `${s.pid}:${s.id}`));
  for (const [k, c] of [...liveCards.entries()]) {
    if (!keys.has(k)) { stopWatching(c.el); c.el.remove(); liveCards.delete(k); }
  }
  if (!liveSel || !keys.has(liveSel)) liveSel = [...keys][0];
  if (liveBig && !keys.has(liveBig)) liveBig = null;
  // Built once and updated in place: moving an <img> would restart its stream.
  let root = document.getElementById("live-root");
  if (!root) {
    for (const c of liveCards.values()) stopWatching(c.el);
    liveCards.clear();
    root = h("div", { id: "live-root" }, h("div", { class: "lgrid", id: "live-grid" }), h("div", { id: "live-steps" }));
    $body().replaceChildren(root);
  }
  const grid = document.getElementById("live-grid");
  for (const s of sessions) {
    const k = `${s.pid}:${s.id}`;
    let c = liveCards.get(k);
    if (!c) { c = card(s); liveCards.set(k, c); grid.append(c.el); }
    fillCard(c, s);
    c.el.classList.toggle("sel", k === liveSel);
  }
  const sel = sessions.find((s) => `${s.pid}:${s.id}` === liveSel);
  document.getElementById("live-steps").replaceChildren(...(sel ? [stepsPanel(sel)] : []));
  if (liveBig) {
    const b = sessions.find((s) => `${s.pid}:${s.id}` === liveBig);
    if (b) {
      const close = () => { liveBig = null; document.querySelectorAll(".zoom").forEach((x) => stopWatching(x)); renderLive(lastLive); };
      if (!document.querySelector(".zoom") || document.querySelector(".zoom").dataset.key !== liveBig) {
        const ov = h("div", { class: "zoom", role: "dialog", "aria-modal": "true", "aria-label": `Layar ${b.id}`, "data-key": liveBig },
          h("div", { class: "zoom-in" }, h("div", { class: "lh" }, h("span", { class: "pf web" }, "web"), h("span", { class: "mono" }, b.id),
            h("button", { class: "btn sm right", onclick: close }, icon("close", "sm"), "Tutup")), screenEl(b, true)));
        ov.addEventListener("keydown", (e) => { if (e.key === "Escape") close(); });
        document.querySelectorAll(".zoom").forEach((x) => { stopWatching(x); x.remove(); });
        document.body.append(ov);
        ov.querySelector("button").focus();
      }
    }
  } else {
    document.querySelectorAll(".zoom").forEach((x) => { stopWatching(x); x.remove(); });
  }
}

async function pollLive() {
  if (!location.hash.startsWith("#/live")) return;
  try {
    renderLive(await api("/api/live"));
  } catch (e) {
    $body().replaceChildren(h("div", { class: "err", role: "alert" }, "Tidak bisa membaca sesi: " + e.message));
  }
  liveTimer = setTimeout(pollLive, 1000);
}

// ---- router ----
function render() {
  if (!S) return;
  const hash = location.hash || "#/live";
  const [, page, sub] = hash.split("/");
  if (page !== "live") { stopLive(); document.querySelectorAll(".zoom").forEach((x) => { stopWatching(x); x.remove(); }); }
  document.querySelectorAll(".rail a").forEach((a) => {
    const on = a.dataset.nav === page;
    a.classList.toggle("on", on);
    if (on) a.setAttribute("aria-current", "page"); else a.removeAttribute("aria-current");
  });
  if (page === "live") {
    if (!liveTimer) {
      $top().replaceChildren(icon("live"), "Live");
      $body().replaceChildren(h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Memuat" }), h("b", {}, "Membaca sesi…")));
      pollLive();
    }
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
