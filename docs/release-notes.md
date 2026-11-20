## What's new in 0.3.0

### New
- **Live for Android and iOS, and "Ambil alih" (take over)** in `fastpilot ui`: Android and iOS screens
  (screenshots from the device, only while the panel is open), and for web, Android and iOS you can take
  a session over, tap, swipe and type yourself while the AI's steps wait, then give it back. The AI's
  next result says the user stepped in, and runs with `test_id` record it. A yellow banner shows a
  takeover on every page; closing the large view gives the session back.
- **Jadwal (schedules) + notifications**: run a test folder or file every day, on weekdays or weekly,
  through the OS scheduler (launchd on macOS, a systemd user timer or cron on Linux), registered only
  when you save. On failure: a Slack-compatible https webhook (the URL is kept as a secret), a
  notification on this computer, and/or a GitHub issue. `fastpilot schedule list | run | remove`.
- **Chat in `fastpilot ui`**: talk to a fastpilot bot driven by your own Claude Code (`claude -p`,
  no API key) with only the fastpilot tools. It can only *propose* schedules and stricter limits, as
  cards in plain Indonesian; nothing changes until you click, and loosening is refused.
- **OpenAI-compatible providers (opt-in)**: `fastpilot run` can be driven by OpenAI, OpenRouter, Groq,
  Ollama or a custom chat-completions API with tool calling (`provider`, `provider_model`,
  `config set-key`, `config test-provider`, or Pengaturan › Model AI). Claude Code stays the default.
  A key is stored in the Keychain / secret service (or an owner-only file) and only sent to the
  server it was saved for; a provider that cannot be used fails the test with the reason instead of
  falling back silently.
- **`--reset`**: every mobile session starts its app fresh (data, permissions, orientation), also when
  the app is opened with `launchApp` after `session start`.
- **Video per flow** in the HTML report (`run --record`): markers per step (red where a step failed),
  a button per flow, and the final screen of passing flows.
- **`--report-github`**: the results as one comment on the pull request (updated, not repeated).
- **`action_timeout`** setting: how long an action waits for its element; text input on phones waits
  longer for the keyboard.
- **Docs**: running fastpilot on another machine over SSH, and lighter iOS simulators with simslim
  (`docs/remote-machine.md`; the SSH parts are marked untested).

### Security fixes
- The limits gaps listed for 0.2.0 are closed: a `point` tap uses the same coordinates as the
  snapshot and checks every element under the point; `pressKey` Enter and `evalScript` obey
  `block_actions` (evalScript is off while `block_actions` is set); the Chrome extension never
  connects without a relay token (extension 0.2.1).
- Takeover socket (`~/.fastpilot/live/<pid>.sock`): served only to a `fastpilot ui` process, a random
  per-takeover token kept in memory is needed for every input, and a takeover ends by itself when the
  page goes away (±45 s) or after 30 minutes.
- Switching provider no longer sends a key to the previous provider's server.
- `fastpilot ui` keeps running when whoever started it stops reading its output.

### Privacy: third-party providers
With an OpenAI-compatible provider, what the model sees goes to that provider: screen snapshots,
step results and errors, which can include local paths such as `file:///Users/<name>/…`. `${NAME}`
secret values stay masked. Use Claude Code (default) or a local Ollama when data must not leave the
machine.

### Known limitations
- Takeover is not a security boundary against code that already runs as you: an agent with shell
  access can start its own `fastpilot ui` (or a program named `fastpilot` run with `ui`) and take a
  session over without a human click; takeover input is not checked by the limits. fastpilot's limits
  protect against an AI that only has fastpilot's tools. A renamed fastpilot binary cannot use
  takeover ("only fastpilot ui may use this socket").
- With `--report-github`, if someone posts a comment carrying fastpilot's marker before the bot does,
  the report can be written into the wrong thread (Low).
- A provider key stored by 0.2.0 has no server recorded; set it again with `config set-key` so it is
  bound to its server. Provider requests follow HTTP redirects (Low).
- A flow with its own `session` step but no session started yet can still fail with
  `unknown session default` (#9).
- A non-point selector is not checked against the limits when the snapshot fails; a point covered
  only by nameless elements passes (Low).
- The report's video player sits in an `<iframe srcdoc>` without `sandbox` (Low).
- Android system-dialog tests can be flaky on a heavily loaded machine.
- Not tested on: Linux (systemd/cron, secret-tool, peer check), physical phones for takeover, real
  hosted providers and Ollama, SSH, the Chrome extension in daily Chrome, VoiceOver.

Install (macOS arm64/x86_64, Linux x86_64):

```bash
curl -fsSL https://raw.githubusercontent.com/autobot-lab/fastpilot/main/scripts/install.sh | sh
npm install -g fastpilot
```

Every archive has a `.sha256` next to it; the install script and the npm package check it.

Earlier releases: https://github.com/autobot-lab/fastpilot/releases (0.2.0: `fastpilot ui`, new limits, `launchApp` arguments, remote devices, Agent Skill).
