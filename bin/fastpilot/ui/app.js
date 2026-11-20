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

function toast(msg, ok) {
  document.querySelectorAll(".toast").forEach((t) => t.remove());
  const t = h("div", ok ? { class: "toast ok", role: "status" } : { class: "toast", role: "alert" }, msg);
  document.body.append(t);
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.remove(), 7000);
}

async function api(path, body) {
  const opts = body === undefined ? {} : { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) };
  let r;
  try {
    r = await fetch(path, opts);
  } catch (_) {
    // The browser's own "Failed to fetch": the fastpilot ui process is not running any more.
    throw new Error("server fastpilot ui tidak menjawab; jalankan lagi `fastpilot ui` di Terminal");
  }
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
// Whether block_actions is in force: the built-in list unless emptied in the file (an empty
// environment variable does not empty it).
function blockingOn() {
  const b = get("block_actions");
  return !(b && b.source === "file" && Array.isArray(b.value) && !b.value.length);
}
// What a switch does now: allow_eval_script not set is off while block_actions is on (option C).
function effectiveOn(key) {
  const s = get(key);
  if (s.value !== null) return !!s.value;
  return key === "allow_eval_script" ? !blockingOn() : true;
}
function toggle(key, label) {
  const s = get(key);
  const on = effectiveOn(key);
  return h("button", {
    class: "tg", role: "switch", "aria-checked": String(on), "aria-label": label, disabled: s.source === "env",
    onclick: () => save(key, !on),
  });
}
// Built-in list of a setting (v0.3.2: block_actions, confirm_actions), in force while it is not set.
const builtinOf = (key) => (S && S.defaults && Array.isArray(S.defaults[key]) ? S.defaults[key] : null);

// A list with a built-in part: the words shown (built-in ones flagged) and what to save after an
// add or a removal. block_actions keeps `default` in the file (built-in words match whole words);
// confirm_actions writes its words out.
function withBuiltin(key, s) {
  const builtin = builtinOf(key);
  const value = Array.isArray(s.value) ? s.value : [];
  if (!builtin) return null;
  const unset = s.source === "default" || s.value === null;
  const own = unset ? [] : value.filter((w) => w.toLowerCase() !== "default");
  const hasDefault = unset || value.some((w) => w.toLowerCase() === "default");
  const shownBuiltin = hasDefault ? (key === "block_actions" || unset ? builtin : []) : [];
  const words = [...shownBuiltin.map((w) => ({ w, builtin: true })), ...own.map((w) => ({ w, builtin: false }))];
  const base = key === "block_actions" ? (hasDefault ? ["default", ...own] : own) : (unset ? [...builtin] : value);
  return {
    unset,
    words,
    add: (v) => [...base, v],
    // Removing a built-in word writes the others out (they then match inside text, like the user's own).
    remove: (it) => it.builtin
      ? [...shownBuiltin.filter((w) => w !== it.w), ...own]
      : (key === "block_actions" && hasDefault ? ["default", ...own.filter((w) => w !== it.w)] : value.filter((w) => w !== it.w)),
  };
}

function chips(key, opts) {
  const s = get(key);
  const bi = withBuiltin(key, s);
  const items = bi ? bi.words.map((x) => x.w) : (Array.isArray(s.value) ? s.value : []);
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
    save(key, bi ? bi.add(v) : [...items, v], v);
  };
  input.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); add(); } });
  // A message about the last attempt goes away as soon as the user changes the text.
  input.addEventListener("input", () => { delete typed[key]; clearToast(); });
  const list = bi ? bi.words : items.map((w) => ({ w, builtin: false }));
  return h("div", { class: "chips" },
    empty,
    list.map((it) =>
      h("span", { class: "chip" + (opts.red ? " x" : "") + (it.builtin ? " dflt" : ""), title: it.builtin ? "kata bawaan" : null }, opts.show ? opts.show(it.w) : it.w,
        locked ? null : h("button", { class: "rm", "aria-label": `Hapus ${it.w}`, onclick: () => save(key, bi ? bi.remove(it) : items.filter((x) => x !== it.w)) }, icon("close", "sm")))),
    preset,
    locked ? null : h("span", { class: "add" }, input, h("button", { onclick: add, "aria-label": opts.addLabel }, icon("plus", "sm"), "Tambah")),
    bi && !locked ? h("span", { class: "acts" },
      h("button", { class: "btn sm", disabled: !bi.unset && !items.length, onclick: () => save(key, []) }, "Kosongkan"),
      h("button", { class: "btn sm", disabled: bi.unset, onclick: () => save(key, null) }, "Kembalikan ke bawaan")) : null);
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

// Who drives `fastpilot run`: Claude Code, or an OpenAI-compatible API (B2).
let provData = null, provBusy = false, provResult = null;
// What the user picked and typed but did not save yet: the page is redrawn (device check done,
// Simpan busy, a refused save) and must not throw it away. The API key is never kept here.
let provDraft = null;
const PROV_LABEL = { claude: "Claude Code (bawaan, tanpa API key)", openai: "OpenAI", openrouter: "OpenRouter", groq: "Groq", ollama: "Ollama (lokal)", custom: "Lainnya (OpenAI-compatible)" };

async function loadProvider() {
  try { provData = await api("/api/provider"); } catch (e) { provData = { error: e.message }; }
  if ((location.hash || "").startsWith("#/settings/model")) render();
}

async function provAction(body) {
  provBusy = true;
  render();
  try {
    const r = await api("/api/provider", body);
    provResult = { ok: true, text: r.message };
    if (body.action === "save") provDraft = null;
    if (body.action !== "test") toast(r.message, true);
  } catch (e) {
    provResult = { ok: false, text: e.message };
    if (body.action !== "test") toast(e.message);
  }
  provBusy = false;
  await loadProvider();
}

function providerGroup() {
  if (!provData) { loadProvider(); return h("div", { class: "grp" }, h("div", { class: "gh" }, icon("plug"), "penggerak test"), h("div", { class: "empty small" }, "Membaca…")); }
  if (provData.error) return h("div", { class: "err", role: "alert" }, "Tidak bisa membaca penyedia: " + provData.error);
  const preset0 = (n) => provData.presets.find((p) => p.name === n) || {};
  const d = provDraft || { provider: provData.provider, base_url: provData.base_url || preset0(provData.provider).base_url || "", model: provData.model || "" };
  const pick = h("select", { class: "sel", "aria-label": "Penyedia model untuk fastpilot run", disabled: provData.env.provider },
    provData.presets.map((p) => h("option", { value: p.name, selected: p.name === d.provider }, PROV_LABEL[p.name] || p.name)));
  const preset = (n) => provData.presets.find((p) => p.name === n) || {};
  const hosted = (n) => ["openai", "openrouter", "groq"].includes(n);
  const url = h("input", { type: "text", class: "mono", value: d.base_url, placeholder: "https://…/v1", "aria-label": "Base URL" });
  const urlHint = h("span", { class: "s" });
  const model = h("input", { type: "text", class: "mono", value: d.model, placeholder: "mis. gpt-4.1-mini, llama3.1", "aria-label": "Id model" });
  const keepDraft = () => { provDraft = { provider: pick.value, base_url: url.value, model: model.value }; };
  url.addEventListener("input", keepDraft);
  model.addEventListener("input", keepDraft);
  const k = provData.key || {};
  const key = h("input", { type: "password", class: "mono", autocomplete: "off", placeholder: k.has ? `tersimpan di ${k.where} · isi untuk mengganti` : "sk-…", "aria-label": "API key" });
  const detail = h("div", { class: "prov-fields" },
    h("label", { class: "fld" }, h("span", { class: "l" }, "Base URL"), url, urlHint),
    h("label", { class: "fld" }, h("span", { class: "l" }, "Model"), model),
    h("label", { class: "fld" }, h("span", { class: "l" }, "API key"), key,
      h("span", { class: "s" }, provData.env.key ? "FASTPILOT_PROVIDER_API_KEY diatur lewat environment dan dipakai lebih dulu."
        : k.has ? `Tersimpan di ${k.where}${k.origin ? `, hanya untuk ${k.origin}` : ""}. Tidak pernah ditampilkan lagi; kunci hanya dikirim ke server tempat ia disimpan.`
        : `Disimpan di ${k.where || "penyimpan kunci sistem"}, tidak di file pengaturan, dan hanya dikirim ke server ini. Ollama lokal tidak butuh kunci.`)));
  // Changing provider changes the server: the preset's URL replaces the previous one (Design must-fix).
  let shownFor = d.provider;
  const sync = () => {
    detail.hidden = pick.value === "claude";
    const p = preset(pick.value);
    if (pick.value !== shownFor) {
      url.value = p.base_url || "";
      shownFor = pick.value;
    }
    url.readOnly = hosted(pick.value);
    urlHint.textContent = hosted(pick.value)
      ? `${PROV_LABEL[pick.value]} selalu memakai server ini. Untuk server lain pilih "Lainnya (OpenAI-compatible)".`
      : pick.value === "custom" ? "Wajib diisi, mis. https://llm.kantor.id/v1. https wajib, kecuali ke komputer ini."
      : "Bawaan Ollama di komputer ini; ubah bila Ollama ada di port lain.";
  };
  pick.addEventListener("change", () => { sync(); keepDraft(); });
  sync();
  const btns = h("div", { class: "btns" },
    h("button", { class: "btn p", disabled: provBusy || provData.env.provider, onclick: () => provAction({ action: "save", provider: pick.value, base_url: url.value, model: model.value, key: key.value }) }, "Simpan"),
    provData.provider !== "claude" ? h("button", { class: "btn", disabled: provBusy, onclick: () => provAction({ action: "test" }) }, provBusy ? "Mengetes…" : "Tes koneksi") : null,
    provData.provider !== "claude" && k.has ? h("button", { class: "btn danger", disabled: provBusy, onclick: () => { if (confirm("Hapus API key yang tersimpan?")) provAction({ action: "delete-key" }); } }, "Hapus kunci") : null);
  return h("div", { class: "grp" },
    h("div", { class: "gh" }, icon("plug"), "penggerak test · fastpilot run"),
    provData.env.provider ? h("div", { class: "banner" }, icon("info"), h("span", {}, "Penyedia diatur lewat environment (FASTPILOT_PROVIDER…), yang menang atas pengaturan ini.")) : null,
    opt("Penyedia", "Claude Code memakai login-mu. API OpenAI-compatible bersifat opsional; berlaku di run berikutnya.", pick, null),
    h("div", { class: "prov" }, detail, btns,
      provResult ? h("p", { class: "prov-result " + (provResult.ok ? "ok" : "bad"), role: "status" }, provResult.ok ? "✓ " : "✗ ", provResult.text) : null));
}

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
      opt("Claude Code", run ? `${run.detail}` : "Memakai login Claude Code milikmu, tanpa API key", status)),
    providerGroup());
}

const NEW_032 = "fp-new-0.3.2-limits";
function pageLimits() {
  const confirmShow = (w) => (w === "default" ? "default (hapus, bayar, beli, …)" : w);
  const emptied = (key) => { const s = get(key); return s && s.source !== "default" && Array.isArray(s.value) && !s.value.length; };
  let seen = false;
  try { seen = localStorage.getItem(NEW_032) === "1"; } catch (e) { /* private mode: show it */ }
  const news = seen ? null : h("div", { class: "banner info new" }, icon("info"),
    h("span", {}, h("b", {}, "Baru di 0.3.2: "), "tombol pembayaran dan hapus akun sekarang diblokir secara bawaan. Kalau memang mau mengetes checkout, hapus katanya di bawah atau klik ", h("b", {}, "Kosongkan"), "."),
    h("button", { class: "rm", "aria-label": "Tutup pemberitahuan", onclick: () => { try { localStorage.setItem(NEW_032, "1"); } catch (e) { /* ignore */ } render(); } }, icon("close", "sm")));
  return h("section", { class: "panel" },
    h("h2", {}, "Batasan & izin", savedNote()),
    h("p", { class: "sub" }, "Aturan ini berlaku untuk semua test: dari Claude Code, Cursor, CLI, maupun ", code("fastpilot run"), "."),
    news,
    h("div", { class: "banner" }, icon("info"), h("span", {}, "Perubahan berlaku mulai run berikutnya. Sesi yang sedang berjalan tetap memakai aturan saat ia dimulai. AI lewat chat hanya bisa memperketat aturan, tidak melonggarkan. Disimpan di ", code(S.path), ".")),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("globe"), "domain yang boleh dibuka"),
      opt("Hanya izinkan domain ini", "Kosong = semua domain boleh. Link, redirect, dan popup ke domain lain diblokir.",
        chips("allowed_domains", { placeholder: "mis. toko.com", addLabel: "Tambah domain", empty: "Semua domain boleh" }), get("allowed_domains"))),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("risk"), "aksi berisiko"),
      opt("Tombol yang selalu diblokir", "Juga dengan confirm: true. Garis putus-putus = kata bawaan.",
        chips("block_actions", { placeholder: "mis. Keluar", addLabel: "Tambah kata yang diblokir", red: true, empty: emptied("block_actions") ? "Dikosongkan: tidak ada yang diblokir" : "Belum ada" }), get("block_actions"),
        h("div", { class: "d" }, "Kata bawaan cocok per kata utuh (\"pay\" kena \"Pay now\", tidak kena \"Paypal\"). Kata tambahanmu cocok kalau teks tombol ", h("b", {}, "berisi"), " kata itu: \"Keluar\" juga memblokir \"Keluarkan\". Selama ada kata di sini, JavaScript bebas (evalScript) ikut mati, kecuali kamu menyalakan \"Jalankan JavaScript bebas\" di bawah.")),
      opt("Tombol yang butuh konfirmasi", "AI harus menulis confirm: true. Cocok per kata. Bawaan: hapus, delete, remove.",
        chips("confirm_actions", { placeholder: "mis. Arsipkan", addLabel: "Tambah kata konfirmasi", show: confirmShow, empty: emptied("confirm_actions") ? "Dikosongkan: tanpa konfirmasi" : "Belum ada" }), get("confirm_actions"))),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("device"), "device & aplikasi"),
      opt("Boleh install / uninstall aplikasi", "Memasang atau menghapus aplikasi di HP, emulator, atau simulator", toggle("allow_app_install", "Boleh install / uninstall aplikasi"), get("allow_app_install")),
      opt("Boleh hapus data aplikasi (reset)", "Mengosongkan data dan login aplikasi, untuk test yang mulai dari awal", toggle("allow_app_reset", "Boleh hapus data aplikasi"), get("allow_app_reset")),
      opt("Boleh pakai HP fisik", "Kalau mati, hanya emulator dan simulator", toggle("allow_physical_devices", "Boleh pakai HP fisik"), get("allow_physical_devices")),
      opt("Boleh ubah setting HP (jaringan, lokasi, izin app, bahasa)", "Contoh: mode offline, GPS palsu, izin kamera, bahasa HP", toggle("allow_device_settings", "Boleh ubah setting HP"), get("allow_device_settings"))),
    h("div", { class: "grp" },
      h("div", { class: "gh" }, icon("lock"), "data & keamanan"),
      opt("Jalankan JavaScript bebas di halaman", "Matikan untuk mencegah AI mengubah halaman di luar langkah test", toggle("allow_eval_script", "Jalankan JavaScript bebas"), get("allow_eval_script"),
        get("allow_eval_script").value === null && blockingOn()
          ? h("div", { class: "d warn" }, "Mati karena daftar blokir aktif (script bisa menekan tombol yang diblokir); nyalakan untuk mengizinkan.")
          : null),
      opt("Boleh ambil alih sesi (Ambil alih di Live)", "Matikan bila agent-mu punya akses shell: input ambil alih tidak diperiksa Batasan. Berlaku untuk proses fastpilot yang mulai setelah ini.", toggle("allow_takeover", "Boleh ambil alih sesi"), get("allow_takeover")),
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
const PLATFORM = { web: "chrome · milik fastpilot", android: "emulator / HP", ios: "simulator / iPhone" };
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
  if (s.taken_over_here) return { text: "Diambil alih olehmu · langkah AI menunggu", cls: "warn" };
  if (s.taken_over_ms) return { text: "Diambil alih di halaman lain · langkah AI menunggu", cls: "warn" };
  const f = s.flow;
  if (!f) return { text: "Siap · menunggu langkah", cls: "" };
  if (f.steps.some((x) => x.state === "blocked")) return { text: "Diblokir Batasan", cls: "bad" };
  if (f.steps.some((x) => x.state === "failed")) return { text: "Gagal", cls: "bad" };
  if (!f.finished) return { text: "Sedang berjalan", cls: "run" };
  return { text: `Selesai · ${f.steps.filter((x) => x.state === "done").length}/${f.steps.length} lolos`, cls: "ok" };
}

// The engine compares in lower case; show the word as the user wrote it in Batasan.
function asWritten(key, w) {
  const list = (get(key) && Array.isArray(get(key).value)) ? get(key).value : [];
  return list.find((x) => x.toLowerCase() === w.toLowerCase()) || w;
}

// Why a guard rail stopped a step, in a few words, from the engine's message.
function blockReason(note) {
  const n = note || "";
  let m = /contains "([^"]+)" \(block_actions(, built-in list)?\)/.exec(n);
  if (m) return `teks berisi "${asWritten("block_actions", m[1])}"${m[2] ? " (daftar bawaan)" : ""}`;
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
  const limits = (i, change) => h("a", { class: "lk bad", href: "#/settings/limits", "data-fk": `limits:${i}` }, change ? "Ubah di Batasan →" : "Lihat Batasan");
  const items = steps.map((x, i) => {
    const n = `${i + 1} · `;
    if (x.state === "running") return h("li", { class: "now" }, h("span", { class: "i" }, icon("play", "sm")), h("span", {}, n, x.label), h("span", { class: "r" }, "sedang berjalan"));
    if (x.state === "done") return h("li", {}, h("span", { class: "i ok", "aria-label": "selesai" }, "✓"), h("span", {}, n, x.label));
    if (x.state === "failed") return h("li", { class: "blk" }, h("span", { class: "i", "aria-label": "gagal" }, "✗"), h("span", {}, n, x.label), h("span", { class: "r" }, x.note || "gagal"));
    if (x.state === "blocked") {
      // block_actions: the button, the word and where it comes from (v0.3.2 design).
      const b = /"([^"]*)"[^"]* contains "([^"]+)" \(block_actions(, built-in list)?\)/.exec(x.note || "");
      const why = b
        ? h("span", { class: "r" }, h("b", {}, "Diblokir Batasan. "), `Teks tombol "${b[1]}" berisi `, code(b[3] ? b[2] : asWritten("block_actions", b[2])), b[3] ? " (daftar bawaan)" : "", ". Halaman tidak ditekan. ", limits(i, true))
        : h("span", { class: "r" }, `diblokir: ${blockReason(x.note)} · `, limits(i));
      return h("li", { class: "blk" }, h("span", { class: "i", "aria-label": "diblokir" }, icon("lock", "sm")), h("span", {}, n, x.label), why);
    }
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
  const mine = !!s.taken_over_here, elsewhere = !!s.taken_over_ms && !mine, off = s.takeover_allowed === false && !mine;
  const label = mine ? "Kembalikan ke AI" : elsewhere ? "Diambil alih di halaman lain" : off ? "Ambil alih dimatikan" : "Ambil alih";
  if (btn.dataset.label !== label) {
    btn.dataset.label = label;
    btn.replaceChildren(icon(mine ? "play" : "cursor", "sm"), label);
    btn.classList.toggle("p", mine);
    btn.disabled = elsewhere || off;
    btn.title = off ? "Dimatikan di Pengaturan › Batasan (allow_takeover)" : "";
    btn.setAttribute("aria-label", mine ? `Kembalikan sesi ${s.id} ke AI` : elsewhere ? `Sesi ${s.id} sedang diambil alih di halaman lain` : off ? `Ambil alih dimatikan di Batasan` : `Ambil alih sesi ${s.id}: langkah AI dijeda`);
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
  if (s.taken_over_here) {
    if (await control(s, "release")) await refreshLive();
    return;
  }
  if (s.taken_over_ms) return;
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
    if (!s || !s.taken_over_here) return;
    e.preventDefault();
    down = fraction(img, e);
  });
  img.addEventListener("pointerup", async (e) => {
    const s = liveSession(key);
    const from = down;
    down = null;
    if (!s || !s.taken_over_here || !from) return;
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
    ? h("button", { class: "btn sm", "aria-label": "Tombol Back di HP", onclick: () => { const s2 = liveSession(key); if (s2) control(s2, "back"); } }, "◁ Back (HP)")
    : null;
  const on = h("div", { class: "steer-on" },
    h("p", { class: "muted small" }, "Klik layar untuk tap, seret untuk swipe. Langkah AI menunggu sampai kamu menekan Kembalikan ke AI atau menutup layar ini; kalau halaman ini ditutup, sesi kembali sendiri setelah ±45 detik."),
    h("div", { class: "steer-row" }, text, h("button", { class: "btn sm", onclick: sendText }, "Ketik"), key_("Enter", "Enter ↵"), key_("Backspace", "⌫ Hapus huruf"), back));
  const take = h("button", { class: "btn sm", "data-fk": `take-zoom:${key}`, onclick: (e) => toggleTakeover(key, e.currentTarget) });
  const bar = h("div", { class: "steer", "data-key": key }, h("div", { class: "steer-head" }, h("span", { class: "steer-state" }), take), on);
  updateSteer(bar);
  return bar;
}

function updateSteer(bar) {
  const s = liveSession(bar.dataset.key);
  if (!s) return;
  const taken = !!s.taken_over_here;
  setTakeLabel(bar.querySelector("[data-fk^='take-zoom:']"), s);
  const close = bar.closest("dialog")?.querySelector(".zoom-close");
  const closeText = taken ? "Kembalikan & tutup" : "Tutup";
  if (close && close.dataset.label !== closeText) { close.dataset.label = closeText; close.replaceChildren(icon("close", "sm"), closeText); }
  const state = bar.querySelector(".steer-state");
  const t = taken ? "Diambil alih olehmu · langkah AI menunggu" : s.taken_over_ms ? "Diambil alih di halaman lain" : "Otomatis · ambil alih untuk klik dan ketik sendiri";
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
      h("button", { class: "btn sm right zoom-close", onclick: () => closeZoom(true, true) }, icon("close", "sm"), "Tutup")),
    screenEl(s, true),
    s.controllable ? steerBar(`${s.pid}:${s.id}`) : null);
  if (s.controllable) steerable(dlg.querySelector(".screen-box"), `${s.pid}:${s.id}`);
  dlg.querySelectorAll(".steer").forEach(updateSteer);
  dlg.dataset.key = `${s.pid}:${s.id}`;
  dlg.addEventListener("cancel", (e) => { e.preventDefault(); closeZoom(true, true); });
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
// `release`: the user closed it; a session they took over goes back to the AI (no hanging takeover).
function closeZoom(restore, release) {
  document.querySelectorAll("dialog.zoom").forEach((d) => {
    const s = release ? liveSession(d.dataset.key) : null;
    if (s && s.taken_over_here) control(s, "release").then(refreshLive);
    stopWatching(d); if (d.open) d.close(); d.remove();
  });
  if (restore && zoomOpener && zoomOpener.isConnected) zoomOpener.focus();
  zoomOpener = null;
}

function livePill(sessions) {
  if (!sessions.length) return h("span", { class: "pill grey" }, "○ Tidak ada sesi berjalan");
  const count = (st) => sessions.filter((s) => s.flow && s.flow.steps.some((x) => x.state === st)).length;
  const running = sessions.filter((s) => s.flow && !s.flow.finished).length;
  const blocked = count("blocked"), failed = count("failed");
  const parts = [`${sessions.length} sesi`, `${running} berjalan`];
  const taken = sessions.filter((s) => s.taken_over_ms).length;
  if (taken) parts.push(`${taken} diambil alih`);
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
  takeBanner(sessions);
  if (document.querySelector("dialog.zoom") && zoomOpener && !zoomOpener.isConnected) closeZoom(false);
}

// A takeover from this page is visible on every page, with the way back.
function takeBanner(sessions) {
  const mine = (sessions || []).filter((s) => s.taken_over_here);
  let el = document.getElementById("take-banner");
  if (!mine.length) { if (el) el.remove(); return; }
  if (!el) {
    el = h("div", { id: "take-banner", class: "take-banner", role: "status" });
    document.getElementById("main").prepend(el);
  }
  const sig = mine.map((s) => `${s.pid}:${s.id}`).join(",");
  if (el.dataset.sig === sig) return;
  el.dataset.sig = sig;
  const names = mine.map((s) => `${s.id} (${s.platform})`).join(", ");
  el.replaceChildren(icon("cursor", "sm"),
    h("span", {}, h("b", {}, "Kamu sedang mengambil alih "), names, ". Langkah AI di sesi itu menunggu."),
    h("a", { class: "btn sm", href: "#/live" }, "Lihat di Live"),
    h("button", { class: "btn sm p", onclick: async () => {
      for (const s of mine) await control(s, "release");
      await refreshLive();
      await globalLive();
      // The banner (and this button) is gone: keyboard focus goes to the page title.
      document.querySelector(".top h1")?.focus();
    } }, icon("play", "sm"), "Kembalikan ke AI"));
}

// Off the Live page: the Live state every few seconds, for the banner (and the takeover's keep-alive).
async function globalLive() {
  if ((location.hash || "#/live").startsWith("#/live")) return;
  try { lastLive = await api("/api/live"); takeBanner(lastLive.sessions); } catch (_) { /* next time */ }
}
setInterval(globalLive, 4000);

async function pollLive() {
  // An empty hash is the Live page too (the first page after the link).
  if (!(location.hash || "#/live").startsWith("#/live")) return;
  try {
    renderLive(await api("/api/live"));
  } catch (e) {
    $body().replaceChildren(h("div", { class: "err", role: "alert" }, "Tidak bisa membaca sesi: ", rich(e.message)));
  }
  liveTimer = setTimeout(pollLive, 1000);
}

// ---- Chat (C4) ----
let chatData = null, chatTimer = null, chatSig = "";
const SUGGEST = ["Tes login di Chrome", "Jadwalkan tests/login.md setiap hari jam 09:00", "Blokir tombol Bayar di semua test"];

async function loadChat() {
  try { chatData = await api("/api/chat"); } catch (e) { chatData = { error: e.message, messages: [], busy: false }; }
  if ((location.hash || "").startsWith("#/chat")) renderChat();
  clearTimeout(chatTimer);
  if (chatData.busy && (location.hash || "").startsWith("#/chat")) chatTimer = setTimeout(loadChat, 800);
}

async function chatSend(text) {
  const t = (text || "").trim();
  if (!t) return;
  try {
    chatData = await api("/api/chat", { message: t });
    const box = document.getElementById("chat-input");
    if (box) box.value = "";
    clearToast();
  } catch (e) { toast(e.message); }
  loadChat();
}

async function chatDecide(id, apply) {
  try {
    const r = await api("/api/chat/decide", { id, apply });
    chatData = r.chat;
    toast(r.message, true);
  } catch (e) { toast(e.message); await loadChat(); }
  renderChat();
  // The clicked button is gone: keyboard focus goes to the card's new state.
  document.querySelector(`.prop[data-pid="${id}"] .prop-state`)?.focus();
}

// `kode` in a message becomes a code span; everything else stays plain text.
function rich(text) {
  return String(text || "").split(/`([^`\n]+)`/).map((part, i) => (i % 2 ? code(part) : part));
}

function proposalCard(p) {
  const head = h("h3", { class: "gh" }, icon(p.kind === "schedule" ? "clock" : "shield"), p.title);
  const rows = p.rows.length ? h("dl", { class: "prop-rows" }, p.rows.map(([k, v]) => [h("dt", {}, k), h("dd", {}, v)])) : null;
  let foot;
  if (p.status === "pending") {
    foot = h("div", { class: "btns" },
      h("button", { class: "btn p", onclick: () => chatDecide(p.id, true) }, p.kind === "schedule" ? "Simpan jadwal" : "Terapkan batasan"),
      h("button", { class: "btn", onclick: () => chatDecide(p.id, false) }, "Batal"));
  } else if (p.status === "applied") {
    foot = h("div", { class: "prop-state ok", tabindex: "-1" }, "✓ ", rich(p.note || "Tersimpan"));
  } else if (p.status === "cancelled") {
    foot = h("div", { class: "prop-state muted", tabindex: "-1" }, "Dibatalkan");
  } else {
    foot = h("div", { class: "prop-state bad", tabindex: "-1" }, rich(p.note || "Tidak bisa dipakai"),
      p.kind === "limits" ? [" ", h("a", { class: "lk", href: "#/settings/limits" }, "Buka Pengaturan › Batasan")] : null);
  }
  const note = p.status === "pending" && p.note ? h("p", { class: "s" }, rich(p.note)) : null;
  return h("section", { class: "grp prop " + p.status, "aria-label": p.title, "data-pid": String(p.id) }, head, h("div", { class: "prop-body" }, rows, note, foot));
}

function chatMsg(m) {
  if (m.role === "user") return h("li", { class: "msg me" }, h("div", { class: "bubble" }, m.text));
  const working = !m.done;
  return h("li", { class: "msg bot" },
    h("div", { class: "who" }, icon("mark", "sm"), "fastpilot bot"),
    m.tools && m.tools.length ? h("ul", { class: "tools", "aria-label": "Yang dikerjakan bot" }, m.tools.map((t) => h("li", {}, icon("play", "sm"), t))) : null,
    m.text ? h("div", { class: "bubble" }, rich(m.text)) : null,
    working ? h("div", { class: "typing" }, h("div", { class: "spin sm", "aria-hidden": "true" }), m.tools && m.tools.length ? " Sedang bekerja…" : " Sedang berpikir…") : null,
    m.error ? h("div", { class: "err", role: "alert" }, rich(m.error)) : null,
    (m.proposals || []).map(proposalCard));
}

function renderChat() {
  const busy = !!(chatData && chatData.busy);
  $top().replaceChildren(icon("chat"), h("h1", { tabindex: "-1" }, "Chat"),
    h("span", { class: "pill" + (busy ? "" : " grey") }, busy ? "● bot sedang bekerja" : "● siap"),
    h("button", { class: "btn sm right", disabled: busy || !(chatData && chatData.messages.length), onclick: async () => {
      try { chatData = await api("/api/chat", { reset: true }); } catch (e) { toast(e.message); }
      renderChat();
    } }, icon("refresh", "sm"), "Percakapan baru"));
  let root = document.getElementById("chat-root");
  if (!root) {
    const input = h("textarea", { id: "chat-input", rows: "2", maxlength: "4000", placeholder: "Tulis ke fastpilot bot… contoh: \"tes login di Chrome\"", "aria-label": "Pesan untuk fastpilot bot" });
    input.addEventListener("keydown", (e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); chatSend(input.value); } });
    root = h("div", { id: "chat-root", class: "chat" },
      h("ul", { id: "chat-log", class: "log", role: "log", "aria-live": "polite", "aria-label": "Percakapan" }),
      h("div", { class: "composer" }, input, h("button", { id: "chat-send", class: "btn p", onclick: () => chatSend(input.value) }, icon("send", "sm"), "Kirim")),
      h("p", { class: "s muted" }, "Bot memakai Claude Code-mu (claude -p) dan tool fastpilot. Jadwal dan batasan hanya berubah setelah kamu klik kartunya; lewat chat batasan hanya bisa diperketat."));
    $body().replaceChildren(root);
  }
  const send = document.getElementById("chat-send");
  if (send) send.disabled = busy;
  const log = document.getElementById("chat-log");
  const msgs = (chatData && chatData.messages) || [];
  const sig = JSON.stringify(msgs) + (chatData && chatData.error);
  if (sig === chatSig) return;
  chatSig = sig;
  const atBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 40;
  if (chatData && chatData.error) {
    log.replaceChildren(h("li", { class: "err", role: "alert" }, "Tidak bisa membaca chat: " + chatData.error));
  } else if (!msgs.length) {
    log.replaceChildren(h("li", { class: "empty" }, icon("chat", "big"), h("b", {}, "Tanya atau suruh fastpilot bot"),
      h("span", {}, "Jalankan test, tanya kenapa gagal, buat jadwal, atau perketat batasan."),
      h("div", { class: "chips" }, SUGGEST.map((t) => h("button", { class: "chip sug", onclick: () => chatSend(t) }, t)))));
  } else {
    log.replaceChildren(...msgs.map(chatMsg));
  }
  if (atBottom || msgs.length) log.scrollTop = log.scrollHeight;
}

function pageChat() {
  if (!chatData) {
    $top().replaceChildren(icon("chat"), h("h1", { tabindex: "-1" }, "Chat"));
    $body().replaceChildren(h("div", { class: "empty" }, h("div", { class: "spin", role: "status", "aria-label": "Memuat" }), h("b", {}, "Membaca percakapan…")));
  } else {
    renderChat();
  }
  loadChat();
}

// ---- Jadwal (C3) ----
let schedData = null, schedBusy = false, schedForm = null; // schedForm: null | { id?, ... }
const DAYS = ["Senin", "Selasa", "Rabu", "Kamis", "Jumat", "Sabtu", "Minggu"];

let schedTimer = null;
async function loadSchedules() {
  schedBusy = true;
  try { schedData = await api("/api/schedules"); } catch (e) { schedData = { error: e.message, schedules: [] }; }
  schedBusy = false;
  const here = (location.hash || "").startsWith("#/schedules");
  // An open form keeps what is typed: the list refreshes after it closes.
  if (here && !schedForm) pageSchedules();
  // A run in progress: look again until it has finished.
  clearTimeout(schedTimer);
  const running = (schedData.schedules || []).some((x) => x.last && (x.last.passed === undefined || x.last.passed === null) && !stale(x.last));
  if (here && running) schedTimer = setTimeout(loadSchedules, 3000);
}

// A run that started hours ago and never finished: its process is gone.
function stale(l) { return Date.now() - l.started_ms > 3 * 3600 * 1000; }

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
  if ((l.passed === undefined || l.passed === null) && stale(l)) return h("span", { class: "bad" }, "Tidak selesai · ", ago(l.started_ms), h("div", { class: "s" }, "Proses run berhenti sebelum selesai (komputer tidur atau dimatikan?)."));
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
  return h("tr", { "data-id": s.id },
    h("td", {}, h("b", { class: "row-name", tabindex: "-1" }, s.name), h("div", { class: "s mono" }, s.id)),
    h("td", {}, code(name), h("div", { class: "s", title: s.tests }, s.tests)),
    h("td", {}, x.when_text),
    h("td", {}, lastRun(x.last)),
    h("td", {}, notifyChips(x)),
    h("td", {}, on),
    h("td", {}, h("div", { class: "acts" },
      h("button", { class: "btn sm", onclick: () => schedAction({ action: "run", id: s.id }, () => toast(`"${s.name}" dijalankan sekarang. Hasilnya muncul di Run terakhir.`, true)).then(() => setTimeout(loadSchedules, 1500)) }, icon("play", "sm"), "Jalankan"),
      h("button", { class: "btn sm", onclick: () => schedAction({ action: "test", id: s.id }, (r) => toast("Tes notifikasi: " + r.sent.join(" · "), !r.sent.some((x) => x.includes("gagal")))) }, "Tes notifikasi"),
      h("button", { class: "btn sm", onclick: () => { schedForm = Object.assign({}, s, { webhook_set: !!x.webhook }); pageSchedules(); } }, "Ubah"),
      h("button", { class: "btn sm danger", onclick: () => { if (confirm(`Hapus jadwal "${s.name}"? Entri di penjadwal sistem ikut dihapus.`)) schedAction({ action: "delete", id: s.id }); } }, "Hapus"))));
}

function schedPanel() {
  const f = schedForm;
  const editing = !!f.id;
  // Each field has its own error line; a refused save points at the field it is about.
  const errs = {};
  const field = (label, input, hint, key) => {
    const err = h("span", { class: "fld-err", role: "alert", hidden: true, id: key ? `err-${key}` : undefined });
    if (key) {
      errs[key] = { err, input };
      // Read again when the user comes back to the field, not only when it appears.
      input.setAttribute("aria-describedby", `err-${key}`);
    }
    return h("label", { class: "fld" }, h("span", { class: "l" }, label), input, hint ? h("span", { class: "s" }, hint) : null, err);
  };
  const name = h("input", { type: "text", maxlength: "80", value: f.name || "", placeholder: "mis. Checkout hari kerja" });
  const tests = h("input", { type: "text", value: f.tests || (schedData && schedData.tests_hint) || "", placeholder: "/path/lengkap/ke/tests atau file .md", class: "mono" });
  const every = h("select", { class: "sel" },
    [["day", "Setiap hari"], ["weekday", "Hari kerja (Senin–Jumat)"], ["week", "Mingguan"]].map(([v, t]) => h("option", { value: v, selected: (f.when && f.when.every) === v }, t)));
  const day = h("select", { class: "sel" }, DAYS.map((d, i) => h("option", { value: String(i + 1), selected: f.when && f.when.day === i + 1 }, d)));
  const dayWrap = field("Hari", day, null, "day");
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
  const whField = field("URL webhook", whUrl, "Disimpan sebagai rahasia (~/.fastpilot/secrets, hanya pemilik) dan tidak ditampilkan lagi.", "webhook");
  const repoField = field("Repo GitHub", repo, h("span", {}, "Ditulis pemilik/repo. Token dari GITHUB_TOKEN / GH_TOKEN, atau Keychain item ", code("fastpilot-github"), "."), "repo");
  const syncNotify = () => { whField.hidden = !wh.checked; repoField.hidden = !gh.checked; };
  wh.addEventListener("change", syncNotify);
  gh.addEventListener("change", syncNotify);
  syncNotify();
  const where = (msg) => {
    const m = msg.toLowerCase();
    if (m.includes("nama jadwal") || m.includes("id jadwal")) return "name";
    if (m.includes("test")) return "tests";
    if (m.includes("jam")) return "time";
    if (m.includes("hari")) return "day";
    if (m.includes("repo")) return "repo";
    if (m.includes("webhook")) return "webhook";
    return null;
  };
  const showError = (msg) => {
    Object.values(errs).forEach(({ err, input }) => { err.hidden = true; input.removeAttribute("aria-invalid"); });
    const at = errs[where(msg)];
    if (!at || at.err.parentElement.closest("[hidden]")) { toast(msg); return; }
    at.err.textContent = msg;
    at.err.hidden = false;
    at.input.setAttribute("aria-invalid", "true");
    at.input.focus();
  };
  const cancel = () => { schedForm = null; clearToast(); pageSchedules(); document.querySelector(".top .btn.p")?.focus(); };
  const save = async () => {
    const schedule = {
      id: f.id || "", name: name.value, tests: tests.value, enabled: f.enabled !== false,
      when: { every: every.value, time: time.value, day: every.value === "week" ? Number(day.value) : undefined },
      notify: { webhook: wh.checked, os: os.checked, github: gh.checked ? repo.value.trim() : undefined },
    };
    const body = { action: "save", schedule };
    if (whUrl.value.trim()) body.webhook = whUrl.value.trim();
    let r;
    try { r = await api("/api/schedules", body); } catch (e) { showError(e.message); return; }
    schedForm = null;
    await loadSchedules();
    toast(editing ? "Jadwal disimpan." : `Jadwal disimpan dan didaftarkan ke ${schedData.backend}.`, true);
    // Focus on the saved schedule in the list (the form and its button are gone).
    document.querySelector(`tr[data-id="${r.id}"] .row-name`)?.focus();
  };
  const panel = h("section", { class: "grp sched-form", "aria-label": editing ? "Ubah jadwal" : "Jadwal baru",
    onkeydown: (e) => { if (e.key === "Escape") { e.preventDefault(); cancel(); } } },
    h("h2", { class: "gh" }, icon("clock"), editing ? `ubah jadwal · ${f.id}` : "jadwal baru"),
    h("div", { class: "form" },
      field("Nama", name, null, "name"),
      field("Test", tests, "Folder berisi test (*.md) atau satu file .md, path lengkap. Dijalankan dengan fastpilot run.", "tests"),
      h("div", { class: "row2" }, field("Kapan", every), dayWrap, field("Jam (waktu komputer ini)", time, null, "time")),
      h("fieldset", { class: "fld" }, h("legend", { class: "l" }, "Kalau gagal (bisa lebih dari satu)"),
        h("label", { class: "ck" }, wh, " Webhook (kompatibel Slack)"), h("div", { class: "sub-fld" }, whField),
        h("label", { class: "ck" }, os, " Notifikasi di komputer ini"),
        h("label", { class: "ck" }, gh, " Issue GitHub"), h("div", { class: "sub-fld" }, repoField)),
      h("div", { class: "btns" },
        h("button", { class: "btn", onclick: cancel }, "Batal"),
        h("button", { class: "btn p", onclick: save }, "Simpan jadwal")),
      h("p", { class: "s muted" }, "Escape menutup panel ini tanpa menyimpan.")));
  return panel;
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
  } else if (page === "chat") {
    chatSig = "";
    pageChat();
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
