## What's new in 0.3.2

Easier to start, safe by default.

### New
- **`fastpilot demo`**: writes a demo folder (demo pages, three example tests, `.mcp.json` with the
  public demo password, and the fastpilot Skill) from the binary alone, so a first test runs without
  the repository: `fastpilot demo && cd fastpilot-demo && claude`, then type
  "Pakai fastpilot, jalankan tests/login.md lalu report". `--android` installs the demo app (built
  into the binary) on the attached device or emulator. An existing folder is never overwritten
  (`--force` writes it again and keeps your own `.mcp.json`, saving the old one as `.mcp.json.bak`).
- **`fastpilot init`**: sets up a project folder in one command: checks the machine, registers
  fastpilot with Claude Code (`claude mcp add`), installs the Skill, adds `tests/contoh.md`, and shows
  the limits in force. Every change asks first (`--yes` agrees to all); outside a terminal it only
  prints the plan; a second run changes nothing. `--global` uses the user scope.
- **Safe by default**: on a new install, buttons for payment and account deletion are always blocked
  (`block_actions`: bayar, pay, beli, buy, checkout, purchase, transfer, buat pesanan, place order,
  hapus akun, delete account; matched as whole words, also through spacing, full-width and zero-width
  tricks, e.g. "Pay now", "Delete my account", "Place your order"), and plain deletes need
  `confirm: true` (`confirm_actions`: hapus, delete, remove), so "Hapus filter" still works.
  Pengaturan › Batasan & izin shows the built-in list with **Kosongkan** and **Kembalikan ke bawaan**.
  The AI can only add to these lists, never empty or replace them.
- **README in English, simpler**: three steps (install → demo → init); the full guide moved to
  `docs/guide.md` (still Indonesian for now). Messages in the terminal for people (`doctor`, `setup`,
  `init`, `demo`, `config list`, `help`) are in Indonesian; what the AI reads (tool results, Skill)
  stays English.

### Upgrading from 0.3.1: what changes for existing tests
- **Flows that tap Bayar / Pay / Buy / Checkout / Purchase / Transfer / Place order / Hapus akun /
  Delete account are now blocked**, also with `confirm: true`, until you change the list. Steps that
  tap "Hapus" / "Delete" / "Remove" need `confirm: true`.
- **`evalScript` is off while a block list is active** (the built-in list counts), because a script
  could press a blocked button. To allow it: `fastpilot config set allow_eval_script true` (only you
  can; the AI cannot), or the "Jalankan JavaScript bebas" switch on the Batasan page.
- **To go back to no blocking, in one line:** `fastpilot config set block_actions ""` (or the
  Kosongkan button), then restart the MCP server. `fastpilot config unset block_actions` restores the
  built-in list. An empty environment variable does not switch the built-in list off.
- Words are matched whole: "Pembayaran" (or "Paypal") is **not** blocked by "bayar"/"pay". Add your
  own words if your app needs them (`default,<word>` keeps the built-in list).

### Known limitations
- The built-in list is a list, not a guarantee: text written with invisible marks inside a word
  (left-to-right marks, combining characters), look-alike letters or math letters can slip past it.
- An environment value such as `FASTPILOT_BLOCK_ACTIONS=","` still empties the list; environment
  and `.mcp.json` are trusted (whoever can write them can also run other programs).
- `demo --force` on a `.mcp.json` that is a symlink copies its target into `.mcp.json.bak` before the
  write is refused; `demo --android` does not check `allow_physical_devices`; `init` counts any
  `fastpilot` entry in `.mcp.json` as registered.
- `demo --android` from a source build needs the demo app built first (release builds include it).
- `config set` validation errors are still in English; `setup --help` and `doctor --help` run the
  command instead of showing help; `config list` shows a mixed "(default (bawaan))" label.
- Carried over: a refused takeover from another installation sometimes shows a misleading message;
  Android WebView and system-dialog tests can be flaky on a heavily loaded machine.
- Not tested: real Claude Code answering `y/N` in a terminal, `npx` detection from the npm registry,
  Linux, physical phones, real hosted providers and GitHub posting, the real Keychain, iOS demo
  (comes with `fastpilot setup ios`, next release).

Install (macOS arm64/x86_64, Linux x86_64):

```bash
curl -fsSL https://raw.githubusercontent.com/autobot-lab/fastpilot/main/scripts/install.sh | sh
npm install -g fastpilot
```

Every archive has a `.sha256` next to it; the install script and the npm package check it.

Earlier releases: https://github.com/autobot-lab/fastpilot/releases (0.3.1: takeover only from the same installation and `allow_takeover`, provider key and redirect fixes; 0.3.0: Live Android/iOS + Ambil alih, Jadwal, Chat, OpenAI-compatible providers).
