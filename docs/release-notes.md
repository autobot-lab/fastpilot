## What's new in 0.3.1

A fix release for the known limitations of 0.3.0.

### Fixes
- **Ambil alih (takeover):** the control socket now serves only a `fastpilot ui` running the **same
  program file** as the fastpilot process that owns the session (peer pid and executable path are
  checked); a program merely named `fastpilot` is refused. New limit **`allow_takeover`** (default on,
  tighten-only, pinned at start; Pengaturan › Batasan): turn it off when your agent has a shell. Run
  `fastpilot ui` the same way as its MCP server (e.g. `npx -y fastpilot ui` with `npx -y fastpilot mcp`).
- **PR comments (`--report-github`):** only a marked comment written by the token's own account is
  edited; a copied marker in someone else's comment is left alone.
- **Provider keys:** a key saved by 0.2.0 (no server recorded) is not sent to a custom or local server;
  run `fastpilot config set-key <provider>` once. Provider redirects are no longer followed (the key and
  the request body never go to the redirect target).
- **Flows with their own `session` step** run without an `open` first (`unknown session default`, #9).
- **Limits:** a tap by text, `id`, `ref` or `role` waits for its element like the tap itself and is
  refused while a rule is on and the element cannot be seen (fail closed), so a "Bayar" button that
  appears late is still blocked.
- **Reports:** the video player iframe is sandboxed (`allow-scripts`, no same origin).
- **UI:** Jadwal messages are all Indonesian; after "Kembalikan ke AI" focus goes to the page title;
  Jadwal field errors are tied to their fields; Chat names `vars` as "Nama secret"; a stopped
  `fastpilot ui` is reported in plain words instead of "Failed to fetch".

### Known limitations
- Takeover is still not a security boundary against an agent that can run the real `fastpilot ui`
  (it has a shell): turn `allow_takeover` off for such agents. With `allow_takeover = false`, watching
  an Android/iOS screen in Live still works; only steering is off.
- After upgrading on Linux, a fastpilot process started before the upgrade refuses takeover until it
  is restarted.
- `--report-github` with a GitHub App installation token can post a new comment on every run (use the
  workflow's `GITHUB_TOKEN` or a personal token).
- A refused takeover from another installation sometimes shows "proses fastpilot itu tidak menerima
  kendali" instead of the "same installation" message.
- With an OpenAI-compatible provider, what the model sees (snapshots, step results, local paths) goes to
  that provider; `${NAME}` values stay masked. Use Claude Code or a local Ollama when data must stay on
  the machine.
- Android system-dialog and WebView tests can be flaky on a heavily loaded machine (as in 0.3.0).
- Not tested on: Linux, physical phones, real hosted providers and Ollama, real GitHub posting.

Install (macOS arm64/x86_64, Linux x86_64):

```bash
curl -fsSL https://raw.githubusercontent.com/autobot-lab/fastpilot/main/scripts/install.sh | sh
npm install -g fastpilot
```

Every archive has a `.sha256` next to it; the install script and the npm package check it.

Earlier releases: https://github.com/autobot-lab/fastpilot/releases (0.3.0: Live Android/iOS + Ambil alih, Jadwal, Chat, OpenAI-compatible providers, `--reset`, video per flow, `--report-github`).
