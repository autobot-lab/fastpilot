# fastpilot

MCP server berupa **satu binary Rust** untuk QA automation full-AI di Web (Android dan iOS menyusul). AI agent
(Claude Code, Claude Desktop, Cursor, dll.) membaca test case bahasa natural, lalu menjalankan banyak langkah
dalam **satu** `run_flow`. Hasilnya berupa snapshot teks ringkas dan diff, bukan view hierarchy mentah.

- CDP langsung lewat satu WebSocket. Tanpa Node, JVM, Appium, chromedriver, atau XPath.
- DSL YAML kompatibel Maestro (`tapOn`, `inputText`, `assertVisible`, `scroll`, `back`) ditambah `openUrl`,
  `evalScript`, `session`, `parallel`.
- Multi-sesi dalam satu `run_flow`: satu proses Chrome, satu `BrowserContext` terisolasi per sesi, blok `parallel:`.
- Auto-wait (actionability, DOM quiet, request yang dipicu aksi), tanpa `sleep`. Stop-early pada langkah gagal
  dengan alasan, kandidat ref, dan snapshot.

Spesifikasi lengkap: [docs/PRD.md](docs/PRD.md). Status: **Phase 1** (core + mcp + web launch mode).

## Build

Butuh Rust 1.88+ (edition 2024) dan Google Chrome/Chromium (atau set `FASTPILOT_CHROME`).

```bash
cargo build --release
```

Binary ada di `target/release/fastpilot` (~5 MB).

## Konfigurasi MCP

**Claude Code** (dari root proyek yang mau dites):

```bash
claude mcp add fastpilot -- /path/to/fastpilot mcp
```

atau `.mcp.json` di root proyek:

```json
{
  "mcpServers": {
    "fastpilot": {
      "command": "/path/to/fastpilot",
      "args": ["mcp"],
      "env": { "DEMO_PASS": "rahasia123" }
    }
  }
}
```

**Claude Desktop** (`~/Library/Application Support/Claude/claude_desktop_config.json` di macOS,
`%APPDATA%\Claude\claude_desktop_config.json` di Windows): blok `mcpServers` yang sama. Claude Desktop tidak
menjalankan server dari folder proyek, jadi pakai URL atau path absolut di `target`/`openUrl`.

| Env | Fungsi |
|---|---|
| `FASTPILOT_CHROME` | path executable Chrome/Chromium |
| `FASTPILOT_HEADFUL=1` | tampilkan jendela browser (default headless); sama dengan `fastpilot mcp --headful` |
| `FASTPILOT_LOG` | filter tracing, mis. `debug` (log ke stderr; stdout dipakai MCP) |
| variabel lain | dipakai flow lewat `${NAMA}`; nilainya tidak pernah dikirim balik ke AI |

## Tools

| Tool | Fungsi |
|---|---|
| `session` | `start` / `stop` / `list` / `devices`; `start` dengan `target` langsung mengembalikan snapshot |
| `run_flow` | jalankan flow YAML (multi-langkah, multi-sesi, `parallel:`); kembalikan timing per langkah + diff per sesi |
| `check_flow` | validasi tanpa eksekusi: syntax, step belum didukung, sesi tak dikenal, aturan parallel, `${ENV}` hilang |
| `snapshot` | snapshot ringkas (`filter`, `full`); jarang dibutuhkan karena `run_flow` sudah mengembalikan state |
| `screenshot` | JPEG base64 |
| `docs` | referensi DSL on-demand: `dsl`, `selectors`, `parallel`, `examples`, `errors` |

## Contoh: login demo dalam satu round-trip

Test case: [tests/login.md](tests/login.md). Halaman demo: [examples/demo/login.html](examples/demo/login.html)
(path relatif di-resolve dari working directory server).

```yaml
- session: { start: web1, platform: web, target: "examples/demo/login.html" }
- inputText: { into: "Email", text: "a@b.com" }
- inputText: { into: "Password", text: "${DEMO_PASS}" }
- tapOn: "Masuk"
- assertVisible: "Budi"
```

Respons `run_flow`:

```
ok 5/5 steps 1.70s
  ✓ 1 [web1] session start web1 (web) 1063ms
  ✓ 2 [web1] inputText "Email" "a@b.com" 78ms
  ✓ 3 [web1] inputText "Password" "${DEMO_PASS}" 84ms
  ✓ 4 [web1] tapOn "Masuk" 83ms
  ✓ 5 [web1] assertVisible "Budi" 385ms
[web1] screen "Dashboard" (file:///.../examples/demo/login.html)
e7 heading "Dashboard"
e8 text "Selamat datang,"
e9 text "Budi"
e10 button "Keluar"
```

Langkah gagal berhenti dini dan mengembalikan kandidat:

```
fail step 1/1 [web1] tapOn "Hapus Akun": not_found: no element matches "Hapus Akun" (timeout 5000ms)
candidates: e10 button "Keluar"
```

Dua sesi paralel (contoh lain di `docs` topic `parallel`):

```yaml
- session: { start: a, target: "examples/demo/login.html" }
- session: { start: b, target: "examples/demo/login.html" }
- parallel:
    - session: a
      steps:
        - inputText: { into: "Email", text: "a@b.com" }
        - inputText: { into: "Password", text: "${DEMO_PASS}" }
        - tapOn: "Masuk"
        - assertVisible: "Dashboard"
    - session: b
      steps:
        - inputText: { into: "Email", text: "salah@b.com" }
        - tapOn: "Masuk"
        - assertVisible: "Email atau password salah"
```

## Bench

```bash
./target/release/fastpilot bench --iterations 30
```

Contoh hasil (M1 Pro, Chrome headless): snapshot p50 1.5 ms, tap→diff p50 83 ms, session start warm 167 ms.
Bench ini mencetak latensi launch Chrome, session start, snapshot, dan tap→diff terhadap halaman demo, lalu
membandingkannya dengan target PRD §14 (snapshot <50 ms, tap→diff <200 ms, session start warm <1 s).

## Test

```bash
cargo test
```

Unit test ada di `crates/core` (DSL, `check_flow`, prune/ref/diff, resolusi selector, engine dengan driver palsu).
`bin/fastpilot/tests/mcp_login.rs` menjalankan `fastpilot mcp` lewat stdio dan menguji login, stop-early, dua sesi
paralel, `check_flow`, `snapshot`, dan `screenshot` terhadap Chrome sungguhan. Test ini di-skip kalau Chrome
tidak ditemukan.

## Layout

```
crates/core      Driver trait, Selector, Snapshot (prune, ref stabil, diff), DSL + check_flow, engine, SessionRegistry
crates/web       CDP client, launch Chrome, WebPage (Driver) per BrowserContext
crates/mcp       rmcp stdio server + tools + docs
crates/android   stub (Phase 2)
crates/ios       stub (Phase 5)
crates/memory    stub (Phase 3: path memory + run reports)
bin/fastpilot    entrypoint: `mcp`, `bench`
examples/demo    halaman login lokal
tests            test case bahasa natural
```

## Belum ada di Phase 1

`launchApp`, `assertNotVisible`, `swipe` (mobile), tool `assert` dan `report`, path memory, attach mode,
`snapshot(coords:)`, screenshot Set-of-Mark, Android, dan iOS. Lihat roadmap di PRD §16.
