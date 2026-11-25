# fastpilot — full guide

The complete guide and technical reference. New here? Start with the [README](../README.md): it has only the
steps you need to get going.

## The first 10 minutes

You need: macOS or Linux, Chrome, and [Claude Code](https://claude.com/claude-code), already signed in.

In a hurry? Run `fastpilot` and follow the [README](../README.md#start-in-2-steps): paste your app's address
in Chat. The steps below use Claude Code in a terminal.

**1. Install** (about 1 minute)

```bash
npm install -g fastpilot
```

**2. Try the demo** (about 3 minutes)

```bash
fastpilot demo && cd fastpilot-demo && claude
```

Type in Claude: *"Use fastpilot, run tests/login.md, then report"* (approve the `fastpilot` server when
Claude asks). Result: ✓ `login-ok` passes, and the report is in `.fastpilot/runs/`. `fastpilot demo` creates
a demo page, a sample test, `.mcp.json` (fastpilot + the public demo password), and the fastpilot Skill in that folder.

**3. Use it in your project** (about 5 minutes)

```bash
cd ~/my-project && fastpilot init
```

`init` checks your computer, registers fastpilot with Claude Code, installs the Skill, creates `tests/example.md`,
and shows the Limits. It asks before each change (`--yes` approves all of them). Change `target` in
`tests/example.md` to your app's URL, then ask Claude to run it.

### If you get stuck

Run `fastpilot doctor` first. It usually shows the cause and the fix right away.

| If you see | Do this |
|---|---|
| `Chrome not found` | `fastpilot setup web` |
| fastpilot does not show up in Claude | `claude mcp list`, then `fastpilot init` again |
| `blocked: … (the built-in blocked list)` / "Blocked by Limits" in Live | on purpose: payment and delete-account buttons are blocked by the built-in limits. Change this in `fastpilot ui` → Settings › Limits & permissions |
| `evalScript is off while buttons are blocked` | the "Run any JavaScript" toggle in Limits & permissions, or `fastpilot config set allow_eval_script true` |
| `looks like a risky action … confirm_actions` | a delete/remove button: write that step with `confirm: true` if you really mean it |
| `This build of fastpilot has no Android helper` | install fastpilot from npm or the release (they include it), or run `agent-android/build.sh` and build fastpilot again |
| `runner not built` | `ios-runner/build.sh` from a checkout (iOS, see below) |
| `variable X is not allowed` | add `X` to `FASTPILOT_VARS` in the MCP configuration |
| Detailed log | add `-e FASTPILOT_LOG=debug` (logs go to the MCP server's stderr) |

### Android

```bash
fastpilot setup android
```

```bash
fastpilot demo --android
```

`setup android` sets up the SDK and the emulator (it asks before each step). `demo --android` installs the demo app
on the connected device or emulator (`--device SERIAL` if there are several). Then, in the demo folder:
*"Use fastpilot, run tests/login-android.md"*. Its last step tries to press "Bayar" and
expects fastpilot to refuse it.

### iOS

iOS needs a Mac with Xcode. Your installed iPhone simulators are used as they are; iPhone testing only
needs a one-time setup: `fastpilot setup ios` builds a small test runner with your Xcode (about 5 minutes).
`fastpilot init` offers it when Xcode is found, and the dashboard's Device page has **Set up iPhone testing**
and a list of your simulators to pick the one tests use (`ios_udid`; without a pick, the running one).
You can also skip it: the first iPhone test of `fastpilot run` (in Terminal, or **Run now** on the Schedules page)
builds the runner once, showing the time so far, then starts the test. A schedule does not start that build: it
says "could not run" and asks you to run one iPhone test by hand first. From a checkout, `ios-runner/build.sh` does the same
(see [Build from source](#build-from-source)), and also builds the demo app below.

**iOS (simulator):**

```bash
xcrun simctl boot "iPhone 16"
```

```bash
xcrun simctl install booted ios-runner/build/Build/Products/Debug-iphonesimulator/FastpilotDemo.app
```

Then ask Claude: *"Use fastpilot on the ios platform, open the app com.fastpilot.demo, log in with a@b.com /
`${DEMO_PASS}`, and check that 'Selamat datang, Budi' shows."* The first iOS session takes about 20–30 seconds
to start the runner.

### Claude Desktop

Add the block below to `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or
`%APPDATA%\Claude\claude_desktop_config.json` (Windows), then restart Claude Desktop:

```json
{
  "mcpServers": {
    "fastpilot": {
      "command": "/Users/NAMA/.local/bin/fastpilot",
      "args": ["mcp"],
      "env": { "FASTPILOT_VARS": "DEMO_PASS", "DEMO_PASS": "rahasia123", "FASTPILOT_DIR": "/Users/NAMA/fastpilot-runs" }
    }
  }
}
```

Claude Desktop does not start the server from your project folder. So use an absolute command path (see
`which fastpilot`; `~/.local/bin` for the install script, `~/.cargo/bin` for `cargo install`), a URL or an absolute
path in `target`, and set `FASTPILOT_DIR` to a results folder you can write to.

### Built-in safety limits

Since 0.3.2, as long as you have not filled in the lists yourself:

- **Always blocked** (`block_actions`): bayar, pay, beli, buy, checkout, purchase, transfer, buat pesanan,
  place order, hapus akun, delete account. Matched on whole words ("Pay now" and "Delete my account" are caught,
  "Paypal" is not), even with `confirm: true`.
- **Needs confirmation** (`confirm_actions`): hapus, delete, remove. The AI must write `confirm: true`.
- **evalScript is off** while the block list is active, because a script could press a blocked button. Turn it on
  yourself with the "Run any JavaScript" toggle or `fastpilot config set allow_eval_script true`.
  The AI cannot turn it on.
- **Clearing the list:** the **Empty it** button in `fastpilot ui` › Settings › Limits & permissions, or
  `fastpilot config set block_actions ""`, then restart the MCP server. To go back to the built-in list:
  `fastpilot config unset block_actions`. The AI (through chat or the `config` tool) can only make limits stricter.

All settings: [Settings](#settings).

## Installation details

### Other ways to install

```bash
curl -fsSL https://raw.githubusercontent.com/autobot-lab/fastpilot/main/scripts/install.sh | sh
```

- npm 12 skips install scripts unless you allow them; then `fastpilot` downloads and checks its program the first time it runs (once; messages on stderr, so MCP is not affected).
- npm and the script download the release archive from [GitHub Releases](https://github.com/autobot-lab/fastpilot/releases)
  and check its SHA-256 before installing. The script installs to `~/.local/bin`. For another location:
  `FASTPILOT_INSTALL_DIR=/usr/local/bin`. For a specific version: `FASTPILOT_VERSION=0.7.3`.
- Without installing: `npx -y fastpilot mcp`.
- Homebrew is not available yet (the tap repo does not exist yet).
- The Android agent is already built into the release binary. iOS still needs Xcode and the runner from a checkout
  (`ios-runner/build.sh`, see [Build from source](#build-from-source)).

### Check and prepare your computer

```bash
fastpilot doctor
```

`doctor` checks each platform and shows ✓/!/✗ with hints on how to fix things:

- **web:** Chrome is found and can start headless.
- **android:** `adb`, the built-in agent, a running device/emulator, an available AVD.
- **ios:** Xcode, simulator, XCUITest runner.
- **run:** Claude Code (`claude`) for `fastpilot run`.

`fastpilot doctor android` (or `web`, `ios`) checks only one platform and exits with a code ≠ 0
if that platform is not ready. `fastpilot doctor` with no arguments shows every platform, but its exit
code depends only on web (≠ 0 if web has a ✗), the same rule as `setup`. In CI,
name the platforms you use, e.g. `fastpilot doctor web android`. To fix what can be fixed automatically (install Chrome with
Homebrew, download the Android image, create the `fastpilot_api35` AVD, start the emulator). On the
dashboard's Device page, when no Android device runs, the emulators already on this computer come first,
each with **Start** (a free port, never 5554; other emulators are not touched); **Add a new test phone**
(`fastpilot_api35`) is the second choice:

```bash
fastpilot setup android
```

`setup` asks about each fix (y/N) before running it. `--yes` approves all of them. Outside a
terminal (CI, an agent) `setup` only prints its plan. Like `doctor`, `setup` exits with a code ≠ 0
if a ✗ remains afterwards (things that cannot be fixed automatically, or were skipped, failed, or only planned)
on the named platforms, or on web if no platform is named.

### Register with Claude Code manually

`fastpilot init` does this for you. To do it by hand, from your project folder:

```bash
claude mcp add fastpilot -e FASTPILOT_VARS=DEMO_PASS -e DEMO_PASS=rahasia123 -- fastpilot mcp
```

```bash
claude mcp list
```

`fastpilot` must show with the status connected. The variables in `FASTPILOT_VARS` (Secrets) are the only
variables a flow may use through `${NAMA}`, and their values always show as `***` in output. To
see the browser while a test runs, add `-e FASTPILOT_HEADFUL=1` (or `fastpilot config set headful true`).

Tested with Claude Code and Claude Desktop. Other MCP clients (Cursor, Codex, Gemini CLI) use the same
method (`fastpilot mcp` over stdio), but they have not been tested.

**Agent Skill** (`fastpilot init` and `fastpilot demo` already install it). [`skills/fastpilot/SKILL.md`](../skills/fastpilot/SKILL.md) (in the
[Agent Skills](https://agentskills.io) format) teaches the agent how to use fastpilot: the `session` →
`run_flow` → `report` sequence, path memory hints, secrets as `${NAMA}`, and the limits it must not work around.
Claude Code loads it automatically when the task matches:

```bash
mkdir -p ~/.claude/skills && cp -R skills/fastpilot ~/.claude/skills/        # every project
```

```bash
mkdir -p .claude/skills && cp -R /path/ke/fastpilot/skills/fastpilot .claude/skills/   # one project
```

Without a checkout: download the folder from GitHub (`skills/fastpilot/`). Other clients that support the Agent
Skills format use the same folder in their own skill location (not tested).

### Write your own tests

Write a Markdown test case like `tests/login.md`: a title, `id`, `platform`, `target`, then the steps and
expected results in plain language. You do not need to write selectors or scripts; Claude builds the flow.
Set passwords once in the dashboard (Settings › Secrets › Set value): they are kept in the key store
for the current app and used only on its own website. In CI, put them in the environment instead
(`-e NAMA=nilai`, then add `NAMA` to `FASTPILOT_VARS`); never in the config file. If you run the same test twice, the second time `session start` returns a *hint* from the
previous run, so the test can finish in a single `run_flow`.

### One test on web, Android and iPhone, side by side

A test whose `platform:` line names two or more platforms (or `all`) runs once on **each** platform, and
`fastpilot run` compares the runs step by step ([tests/login-side-by-side.md](../tests/login-side-by-side.md)):

```markdown
# Login on web, Android and iPhone
id: login-side-by-side
platform: web, android, ios
target web: examples/demo/login.html
target android: com.fastpilot.demo/.LoginActivity
target ios: com.fastpilot.demo
- Open the login screen and check that "Masuk ke Akun" shows
- Check that there is a link for a forgotten password, and note its exact text
- Sign in with a@b.com / ${DEMO_PASS} and tap "Masuk"
- Check that the Dashboard shows "Selamat datang, Budi"
```

- **Targets per platform:** `target web:`, `target android:` and `target ios:` lines. A plain `target:` line
  is used for every platform without its own line.
- **One run per platform:** each runs as `<id>-web`, `<id>-android`, `<id>-ios` (its own path memory and
  report) on a device from the device queue: one device runs one test at a time, the other platforms go on.
  `--workers 3` runs the three at once.
- **A platform without a device is not skipped:** its column says *Not run: no iPhone simulator* with what to
  do, and the run is not a pass.
- **The report** (`--html`, and the report in History) starts with **Side by side**: a table with the steps as
  rows and the platforms as columns; each cell has the screen after the step and Passed / Failed / Not run in
  words. A row is marked *Fails on Android* when a step fails on some platforms and passes on the others (that
  column says *Failed at step 3*), and *Differs on iPhone* when the text a check found is not the same everywhere
  (letter case and spacing aside). A platform that could not start (its session or helper failed) says *Could not
  run* with the reason, and is not counted as a failure of your app. The same text in
  another place is not compared. The terminal prints the same summary after the run.
- **Live** shows one summary row per such test, with a tile per platform: waiting (and why), the step it is
  on, done, failed or not run.
- **Chat:** "run the login test on all three" proposes the same kind of test in the "Save test & schedule"
  card.
- This is different from `sessions: web (…), android (…)` ([tests/login-cross-platform.md](../tests/login-cross-platform.md)):
  that is **one** run that uses several devices at the same time.

The iOS demo app has one label planted on purpose for this example: its link says "Lupa kata sandi?" where the
web page and the Android app say "Lupa password?", so the side-by-side report of the demo login shows one
difference.

## Features

- **Web:** CDP directly over one WebSocket (no Node/chromedriver). *Launch* mode (Chrome owned by
  fastpilot, one `BrowserContext` per session) or *attach* mode (the user's own Chrome).
- **Android:** the adb server protocol written in Rust, plus an instrumentation agent (UiAutomation)
  embedded in the binary. WebView content is merged into the snapshot and controlled over CDP.
- **iOS (simulator and iPhone):** fastpilot's own XCUITest runner (`ios-runner/`), built from
  source, without Appium/WebDriverAgent. WKWebView content goes through Web Inspector.
- A Maestro-compatible YAML DSL, plus `openUrl`, `evalScript`, `session`, `parallel`, `pressKey`,
  `selectOption`, `uploadFile`.
- **Reliable:**
  - Auto-wait without `sleep` (actionability, DOM/a11y quiet, navigation, and requests triggered by an action).
  - Stable refs: a ref follows its element and never moves to another element.
  - Stop-early with a reason, a category, and candidates.
  - JS dialogs are handled automatically.
- **Path memory:** a flow that passes is saved as a *hint* so the next run can replay it in 1 round-trip.
  Evidence from each run is stored in `.fastpilot/runs/`.

Specification: [docs/PRD.md](PRD.md). Improvement research: [docs/research-improvements.md](research-improvements.md).

## App map: which screens have a test (fastpilot 0.6)

Open **Map** in the dashboard (between Schedules and Memory), give the start address (a web address
or a file of your site) or the app on the emulator, and press **Map this app**. From a terminal:
`fastpilot map <address|file|app> [--android|--ios] [--screens N] [--minutes M]`.

fastpilot walks through the app like a visitor and draws every screen it finds, then colours each
screen by its saved tests: **Has a test** (green), **Test failing** (red; a failing test still counts
as a test), **No test yet** (grey, dashed). Click a screen for its picture, the tests that reach it,
where it leads, and **Make a test for this screen** (opens Chat with the request written in; you
still decide on the Save test card). **List** shows the same as a table for keyboards and screen
readers.

- **Read-only.** On the web a guard in the browser refuses every request that is not a plain read
  (anything but GET and HEAD, such as a form post or a background save) and every page load outside
  your app, before it is sent; the result says how many it blocked. It never types, presses links,
  tabs and menu items, and only buttons that move around (Next, Menu, More, Details, Help…); it
  never presses a blocked word (Pay…), a word that asks for confirmation (Delete…) or a word that
  could change something (Sign out, Send, Save, Allow…), and checks a link's address first (a leave,
  delete or one-time link is not opened). Every button and link it skipped is named. It stays inside
  your app (the same site or folder; on a phone, the app itself: a system dialog is closed with Back,
  never explored). On a phone there is no network guard: only the button rules apply.
- **What the guard cannot tell apart.** Read-only means no request that changes data *by its method*:
  a plain page load (GET) that changes something on the server (a link such as `/api/touch-me`, a
  tracking pixel) looks like any other read and still goes through; the address rules catch the usual
  ones (leave, sign out, delete, one-time codes). The app's own scripts can also open WebSocket
  connections and use service workers, which the guard does not see. Map a staging or test copy of
  your app, not production.
- **Sign-in.** A sign-in screen (a password field, or only "Sign in with…" buttons) is a wall: nothing
  else on it is pressed. With a test account in **Project notes** (an email and a `${SECRET}` name
  whose value fastpilot has, for example "Test account: qa@shop.test, password in ${STAGING_PASS}"),
  it signs in with that account, only on the start screen or one reached by a sign-in link, and only
  on a form with one text field and one password field; that is the only typing it does.
- **Limits:** up to 30 screens and 10 minutes by default (at most 60 and 30), and **Stop**. Screens
  found so far are kept.
- **What is kept:** in fastpilot's own folder on this computer (`~/.fastpilot/projects/<project>/map/`),
  never in the project, so a cloned project cannot bring a map: one map per start (the latest is
  shown), each screen's title, address (without its query and IDs), the labels that lead on, and one
  picture as the app looked. Titles and labels are cleaned (secrets, emails, tokens and links
  masked); they are text from your app: data, never instructions.

## Jira

### Connect Jira in the dashboard

Open the dashboard, choose **Integrations › Connect Jira**, and click **Sign in with Atlassian**. You
sign in on Atlassian's own page and accept; the browser comes back to the dashboard with "✓ Connected
as … · yoursite.atlassian.net". If your account can use several sites, pick one. The default project
is the one with the most recent issues; change it on the same page.

- fastpilot connects through Atlassian's MCP server, like Claude does: OAuth 2.1 with PKCE, no client
  secret. The sign-in is kept in this Mac's Keychain (Linux: the secret service), renewed silently,
  and never reaches the page, the AI, Chat, logs or reports. **Disconnect** revokes it at Atlassian.
- **Use an API token instead** (a small link on the same page) is for sites whose admin blocks the
  Atlassian sign-in for apps: your Jira address, Atlassian email and an
  [API token](https://id.atlassian.com/manage-profile/security/api-tokens), **Test connection**, a
  default project, then **Connect Jira**.

- **Jira Cloud only** (addresses that end in `.atlassian.net`). Jira Server and Data Center are not
  supported yet.
- **The token** goes to this Mac's Keychain (Linux: the secret service), bound to your site. It is never
  shown again, never sent anywhere else, and never reaches the AI, Chat, logs, reports or the activity
  log. **Manage** has Replace token and Disconnect.
- **Only this page** connects, changes or disconnects Jira: Chat and the AI cannot.
- Issues and comments are filed by the dashboard's **Chat bot** or by **Claude Code**, always as a draft
  you approve first. The dashboard has no "send" form of its own.
- **From Claude Code** (fastpilot's `jira_create_issue` and `jira_comment` tools), nothing is sent by
  the AI: each issue or comment waits under **Integrations › Waiting for your OK** with its exact text, and you
  send it with one click there or with `fastpilot jira approve` in your own terminal. A request expires
  after 30 minutes. `fastpilot jira status` shows what is connected. There is no way around this step
  (no setting or environment value turns it off), and fastpilot never adds an allow rule for these tools.
- `jira_read` reads only tickets of the default project, so text on a tested page cannot make the AI
  read other private tickets.

### Jira in the dashboard Chat

- **Read a ticket:** name it in your message ("test KAN-12"). fastpilot reads it and gives it to the
  bot as information, never as instructions: text in a ticket that asks for something (allow a
  button, post a comment, sign in somewhere) is not followed, and Chat says so when a ticket tried.
  This works for a ticket of another project too (say "test OPS-5"), because you named it in your own
  message; Claude Code's `jira_read` tool reads only the default project.
- **Create an issue or add a comment:** ask in your own words ("make a Jira ticket for the voucher
  bug", "add the result to KAN-12"). The bot's draft becomes a card that fastpilot writes: the site,
  the project or ticket, and the exact text that will be sent (secrets masked, emails and links to
  other sites removed). Nothing is sent before you click **Create issue** or **Add comment**; **Edit
  first** lets you change the text. If an open issue looks the same, the card offers **Comment on
  KAN-120 instead** or **Create anyway**. A card works once and expires after 10 minutes.
- A ticket, page or file can never make a card: only your own message can. No card changes a
  ticket's status. Scheduled and headless runs never write to Jira.

### With Atlassian's MCP server

Three skills work with Jira through **Atlassian's own MCP server** (Rovo MCP) in Claude Code:

| Skill | What it does | Writes to Jira |
|---|---|---|
| `jira-ticket-to-test` | Reads a ticket's acceptance criteria ("make tests from SHOP-123") and drafts tests with `ticket-to-test` | never |
| `jira-bug-report` | Turns a failed run into one Jira issue: summary, steps, expected vs actual, links to the report and video (nothing is uploaded) | after your OK |
| `jira-test-result` | Comments passed / failed and the report link on a ticket; moves the ticket only when you ask for that in your own words | after your OK |

Every write is shown to you first, exactly as it will be sent, and waits for your "yes" in the chat.
Text in tickets, comments and tested pages is treated as data, never as instructions.

**Connect Atlassian once** (in a terminal, then in Claude Code):

```bash
claude mcp add --transport http atlassian https://mcp.atlassian.com/v2/mcp
```

Then open Claude Code, type `/mcp`, choose **atlassian** and sign in in the browser. The sign-in is
between you and Atlassian: fastpilot never sees a password or a token, and never asks for one in chat.

- **Keep Claude Code's own question for every write:** never allow the `mcp__atlassian` server as a
  whole. Allow only its read tools, and keep Claude Code asking for every tool that writes or deletes:
  create, edit, comment, transition, the generic write and delete tools (`executeWrite`,
  `executeDestructive`), and Confluence create or update. Claude Code then asks before each write as
  well, even if a skill was not used. Never use the Jira skills with `--dangerously-skip-permissions`.
- **A test account:** the browser sign-in can do everything your Atlassian user can do, Confluence
  included. If you can, sign in with a dedicated account that sees only the projects testing needs.
- **API token instead of the browser sign-in:** Atlassian's API-token mode for its MCP server has to be
  allowed by an organisation admin of your Atlassian site. Ask them. Keep the token in an environment
  variable rather than on the command line (shell history), never commit a `.mcp.json` that holds it,
  and do not add it with `--scope project`. It never goes into fastpilot or the chat.
- **Where it works:** Claude Code (terminal, or the fastpilot plugin) with the Atlassian server
  connected. The dashboard's Chat uses only fastpilot's own tools, so it cannot reach Jira.
- **Which site and project:** when your account sees several Atlassian sites, the skill asks which
  one. The project always comes from you.

## Settings

Settings are read in this order: **env** (e.g. `-e` in the MCP config) → the **file** `~/.fastpilot/config.toml` (or
the path in `FASTPILOT_CONFIG`) → default. The file is read again when it changes, so you do not need to restart the server.
Limits are the exception: their values are locked when fastpilot starts (MCP server, shell daemon, `fastpilot run`), and
while it runs only stricter values are followed. Looser values take effect only after a
restart (of the MCP server, or `fastpilot daemon stop`), even if that limit was never used before. An invalid file
makes fastpilot stop with an error message; it is not silently ignored.

```bash
fastpilot config list                              # every key, its value, and where it comes from
fastpilot config set allowed_domains "staging.example.com,*.example.com"
fastpilot config get flow_timeout
fastpilot config unset allowed_domains
fastpilot config path
```

```toml
# ~/.fastpilot/config.toml
allowed_domains = ["staging.example.com", "*.example.com"]
headful = true
flow_timeout = 600
```

**In the browser:** `fastpilot ui` opens the dashboard (each agent has Chat, Live, Skills, Schedules, History, Memory, Map,
Model and About; then Device and Settings) in your default browser (only
`127.0.0.1`, random port; `--no-open` only prints the link, `--port N` picks the port, `--help` shows help).
The link holds a **one-time** code that is valid for 10 minutes. Once opened, the code is swapped for a session cookie
and stops working (run `fastpilot ui` again for a new link). Run `fastpilot ui` yourself,
not through an AI agent: anyone who opens the page can loosen the limits. If
`config.toml` is broken, the Settings page shows the error and saves nothing until the file
is fixed.
Settings has the AI model, Limits & permissions, Secrets (names only, their values never show; **Replace** and
**Remove**), Security (remembered browsers) and Integrations (Jira, webhook, GitHub). The Device page has the `doctor`
results with a Check again button. Changes made on this page are user changes: they may
loosen limits, they are written atomically to the file (mode 600), and they apply from the next run. All assets and fonts
are inside the binary, so this page works without internet.

**Live panel** (`fastpilot ui`, Live menu): the sessions currently running from the MCP server (Claude Code),
the CLI, and `fastpilot run`, each with its flow steps (✓ done, ▶ running, 🔒 blocked
or about to be blocked by Limits) and its screen, live, only while the panel is open:

- **Web** (Chrome owned by fastpilot): a screencast over a separate DevTools connection, about 2 fps for the
  thumbnail and 8 fps when enlarged.
- **Android and iOS**: screenshots from the device (the same agent/runner as the `takeScreenshot` step),
  1 fps for the thumbnail and 3 fps when enlarged. This also works for physical phones and adb on another computer.
- Your own Chrome (attach mode) is not shown; its steps are still visible.

**Take over.** The *Take over* button on a session card pauses that session's automatic steps and opens a large screen:
click to tap, drag to swipe, the *Type* field for text in the active field, the Enter/Delete buttons (and
*Back* on Android/iOS). The AI's next step waits until you press *Give back to the AI*. Then the next
`run_flow` result for that session tells the AI that a user took over for a while (how long, how many
taps/texts). With a `test_id`, this note also goes into `steps.jsonl` and the run report. Limits still apply
to all automatic steps, but the input you type yourself is not checked by Limits.

- Input is accepted only from the `fastpilot ui` page (session cookie, Origin, JSON). The page passes it on to
  the fastpilot process that owns the session through the socket `~/.fastpilot/live/<pid>.sock` (folder 0700, socket 0600).
- That socket only serves a `fastpilot ui` process from the **same program file** as the fastpilot process
  that owns the session: the pid and program path on the other end are checked on every connection, so scripts, agents, or other programs
  that are merely named `fastpilot` are refused (for frames too). As a result, `fastpilot ui` and the MCP server must
  come from the same installation (e.g. both from npm), and a renamed binary cannot use Take over.
  Run `fastpilot ui` the same way as its MCP server: if MCP is registered as
  `npx -y fastpilot mcp`, open the page with `npx -y fastpilot ui`. Each take-over gets a random token that
  exists only in the memory of the fastpilot process and the `fastpilot ui` that started it (not in a file, not sent to the
  browser). Taps, typing, and Give back must carry that token, so another page cannot also
  control or close someone else's take-over.
- A take-over ends on its own if the page stops sending a heartbeat for about 45 seconds (tab closed,
  laptop asleep) or after 30 minutes. Closing the large screen (Close / Escape) also gives the session back. While
  a session is taken over, a yellow marker shows on every `fastpilot ui` page with a *Give back to the AI* button.
- **Remaining risk, to be honest:** the checks above tell `fastpilot ui` apart from other processes of the
  same user, but they are not a security boundary against code that already runs as you. An agent with
  shell access (Bash) can run `fastpilot ui` itself, read its one-time link, and then take
  over without a human click; take-over input is not checked by Limits. Such an agent can also
  get around Limits in other ways (running Chrome or `adb` directly). fastpilot's Limits protect
  against an AI that only holds the fastpilot tools. For an agent with a shell, restrict its shell access in that agent's
  app, and turn off Take over: `fastpilot config set allow_takeover false` (or Settings › Limits ›
  *May take over sessions*). Like other limits, this value is locked when the fastpilot process starts, so
  turning it back on takes effect only after a restart.
- Time spent while taken over still counts toward the flow time limit (`FASTPILOT_FLOW_TIMEOUT`).
- macOS and Linux only (Unix socket).

Session data is in `~/.fastpilot/live/` (only its owner can read it). The DevTools address is never
sent to the browser.

**Schedules** (`fastpilot ui`, Schedules menu): run tests (`fastpilot run` on the `tests/` folder or one
`.md` file) every day, on weekdays, or weekly at a set time (this computer's time).

- **One click (v0.7):** three buttons run all of the app's tests by themselves: **Every 2 hours (8–20)**,
  **Every night** (2:00) and **Before each release**. A button that is on says **On · Turn off**, so a second
  click never makes it twice, and a click answers **Created · Undo**. Each comes with a Mac notification
  when something fails, and is named for you ("All tests · every night").
- **Before each release** has no time: it runs when you ask Chat "Can we release?" (or press Run now).
- A test card in Chat has the same three buttons: "Save and run it by itself".
- **Before and after a release (v0.7):** **Check before release** asks Chat "Can we release?", which runs the
  tests and a ship check of the app's own address. **Watch for 2 hours after a release** is one switch: every
  10 minutes for 2 hours, the first run is the "before", and you hear only about a problem that is new since then.
- `fastpilot ship-check` with no page checks the current app's address (Add app in the dashboard).
- **Other times** (New schedule) has the full form: which tests (picked from the list, nothing to type), other
  times, a webhook (a Slack-compatible URL) and GitHub messages (the GitHub repo is filled in from the
  project's own repo).

- Schedules are stored in `~/.fastpilot/schedules.json` and registered with the system scheduler **only when you
  save them**: a launchd LaunchAgent on macOS (`~/Library/LaunchAgents/dev.fastpilot.schedule.<id>.plist`),
  a systemd user timer on Linux, or crontab if there is no systemd. Turning off or deleting a schedule
  removes that entry. The entry contains only `fastpilot schedule run <id>`; the test path and notifications are in the
  JSON file.
- **On failure**: a Slack-compatible https webhook (`{"text": ...}`), a notification on this computer,
  and/or a GitHub issue in `pemilik/repo` (token from `GITHUB_TOKEN`/`GH_TOKEN`, or the Keychain item
  `fastpilot-github`: `security add-generic-password -s fastpilot-github -a fastpilot -w`). The
  *Test notification* button sends a sample message.
- The webhook URL is treated as a secret: it is stored in `~/.fastpilot/secrets/` (owner only), not
  in the schedules file, and the page shows only its host.
- The computer must be on and you must be logged in when the schedule runs (launchd runs missed ones when the computer wakes
  from sleep; systemd uses `Persistent=true`). The HTML report for each run is in
  `~/.fastpilot/schedules/reports/`, the log in `~/.fastpilot/schedules/<id>.log`.
- From the terminal: `fastpilot schedule list`, `fastpilot schedule run <id>`, `fastpilot schedule remove <id>`.
- **Release watch** (after a release): run the tests every 5, 10, 15, 20, 30 or 60 minutes for 1 to 24 hours.
  - The first run starts right away and is the "before".
  - After that you hear only about a problem that is new since then (a test that passed before and fails
    now, or the tests no longer running at all), once per problem, on the schedule's notifications.
  - A `shipCheck` page that got slower than before the watch began counts as a new problem too
    (worked out again from the measured numbers each time)
    ("Speed · /checkout: Loads in 2.8 s, up from 2.0 s (41% slower)"), with the same rules as below.
  - When the hours are up, the watch removes itself from the system scheduler and the list says how it went
    ("Release watch ended: no new problem since the release").
  - Start one from Schedules › New schedule › *Release watch*, from Chat ("watch the release every 10 minutes
    for 2 hours"), or from the terminal:
    `fastpilot schedule watch tests/ --every 10 --for 2 [--github owner/repo] [--os]`.
  - Stop one early by turning it off or deleting it.

**Chat** (`fastpilot ui`, Chat menu): ask the fastpilot bot to run tests, explain failures, create
schedules, or make limits stricter, in plain language. Each message is answered by your own Claude Code
(`claude -p`, your login, no API key) with only the fastpilot tools, the same as `fastpilot run`.
The conversation continues with `--resume`. Sessions the bot opens appear in Live and can be taken over.

- The bot **cannot** change settings or schedules by itself: the `config` tool is not available to it. It
  only makes proposals, and a proposal shows as a card (New schedule, or a settings change). Most changes wait
  until you click *Save the schedule* or *Apply*; one more blocked word applies at once, with *Undo*.
- A proposal that **loosens** a limit (e.g. a larger `max_steps`, allowing a blocked button) gets a red card only
  when your own message asks for it ("allow the Pay button"); it changes only after you type its word to confirm.
  If the request came from a tested page, the chat says "A page tried to change a limit"; if your message said no
  ("don't allow the Pay button"), it says the limit was kept as it is. You can always loosen limits in
  Settings › Limits.
- "Remember that login uses a code" gives an **Add a project note** card that quotes your message. The note must be
  your own words from that message; nothing is saved until you click *Add note*, and every agent reads it from the
  next run (Memory › Project notes). A note the bot got from a tested page is never a card: the chat shows "A page
  tried to give the agent a rule" instead. Hidden characters, passwords, tokens and codes are refused.
- Project notes reach `fastpilot run`, Chat (an open conversation gets your changes with your next message) and
  agents on another provider. Claude Code in Terminal reads them when its fastpilot starts: restart it after you
  change a note.
- Webhook URLs and tokens never go through chat: a schedule from chat that asks for a webhook is saved without
  the webhook, and you then fill in the URL in Schedules › Edit.
- Model: `model_explore` (see Settings › AI model).

Limits and permissions you can set (all are also `fastpilot config` keys):

| Key | Meaning |
|---|---|
| `allowed_domains` | web sessions only open these hosts |
| `confirm_actions` | a tap on an element with a name like this needs `confirm: true` (matched per word). **Built-in (since 0.3.2, as long as you have not filled it in):** `hapus`, `delete`, `remove`. `[]` = no confirmation |
| `block_actions` | a tap on an element whose text **contains** one of these words is always refused, even with `confirm: true` ("Bayar" also blocks "Lanjut ke Pembayaran"). **Built-in (since 0.3.2, as long as you have not filled it in):** `bayar`, `pay`, `beli`, `buy`, `checkout`, `purchase`, `transfer`, `buat pesanan`, `place order`, `hapus akun`, `delete account`, matched on whole words ("pay" catches "Pay now", not "Paypal"); `default` in the list = those built-in words. To clear it: `fastpilot config set block_actions ""` or the **Empty it** button in `fastpilot ui` (takes effect after the MCP server restarts; an empty env variable does not clear it). To go back to the built-in list: `fastpilot config unset block_actions`. Text is normalized first (invisible characters, fullwidth letters, odd spaces), and a built-in phrase may have one word in between ("Delete my account"). A coordinate tap (`point`) is checked against the element at that point; if the element cannot be determined, the tap is refused while `block_actions`/`confirm_actions` is active. This also covers `pressKey Enter`/`Space` that could press such a button (from an input field: while a blocked button is on screen), and `evalScript` is turned off while `block_actions` is active |
| `allow_app_install` | the `installApp` / `uninstallApp` steps (built-in: allowed) |
| `allow_app_reset` | `clearState` / `clearKeychain` (built-in: allowed) |
| `allow_physical_devices` | sessions may use physical phones; `false` = emulators/simulators only (built-in: allowed) |
| `allow_device_settings` | `setNetwork`, `setLocation`, `setPermissions`, `setLocale` on Android/iOS (built-in: allowed) |
| `allow_eval_script` | the `evalScript` step. Built-in (since 0.3.2): **off while `block_actions` is active** (including the built-in list), because a script could press a blocked button; allowed if the block list is cleared. `true` = always allowed (user only: `fastpilot config set allow_eval_script true` or the "Run any JavaScript" toggle; the AI cannot turn it on) |
| `allow_takeover` | the user may take over sessions in `fastpilot ui` (built-in: allowed; turn it off for agents that have a shell) |
| `upload_dirs` | `uploadFile` only takes files from these folders (empty = anywhere) |
| `state_expire_days` | the maximum age of a saved login (built-in: 30 days) |
| `max_steps` / `test_timeout` | `fastpilot run`: maximum AI tool calls per test (built-in: 30) / minutes per test (built-in: 30) |

Through chat, the agent uses the `config` tool with these rules:

- **Limits** (the table above, plus `vars`): the agent may only make them stricter (add words/domains,
  turn off permissions, lower limits). Only the user can loosen or remove them, through
  `fastpilot config` or `fastpilot ui`.
- **Programs, locations, and other computers** (`chrome`, `claude`, `dir`, `ios_runner`, `adb_server`, `chrome_endpoint`):
  user only. A running fastpilot uses the values from when it started; file changes take effect after a restart.
- **Everything else** (`headful`, `model*`, `flow_timeout`, `action_timeout`, `retry_budget`, `breaker`, default device,
  `memory`, `daemon_idle`): free to change. `action_timeout` (seconds, built-in: 5) is the limit for one action (tap, type,
  select, scroll); `inputText` on Android/iOS gets 3× that value because the keyboard can be slow to appear. You can also set it with the env
  `FASTPILOT_ACTION_TIMEOUT` (env wins over the file).
- Secrets (`FASTPILOT_STATE_KEY`, relay token) are never read from the file, only from env. The extension relay
  token is generated randomly for each session; `FASTPILOT_RELAY_TOKEN` is only for browsers without native messaging.
- During `fastpilot run` (with nobody in front of the screen), the `config` tool is not available.
- These limits protect against an agent that only uses the fastpilot tools. A running fastpilot
  process (MCP server, daemon, `run`) locks the limits and the "user only" keys from the moment it starts, so a file
  changed while it runs has no effect. But an agent that also has shell or file access
  (e.g. Bash in Claude Code) can change the file and then restart fastpilot (`fastpilot daemon stop`).
  That is a design boundary: set limits through **env** in the MCP configuration, and deny that access in the agent's permissions,
  e.g. in Claude Code `"deny": ["Bash(fastpilot config:*)", "Bash(fastpilot daemon:*)", "Edit(~/.fastpilot/**)"]`.

Key name = env name without `FASTPILOT_`, in lowercase (`FASTPILOT_FLOW_TIMEOUT` → `flow_timeout`).

## Build from source

You need Rust 1.88+ (edition 2024) and Google Chrome/Chromium. For Android and iOS, the on-device artifacts
are built from source (no downloads):

```bash
git clone https://github.com/autobot-lab/fastpilot.git && cd fastpilot
```

```bash
agent-android/build.sh
```

```bash
ios-runner/build.sh
```

```bash
cargo install --path bin/fastpilot --locked
```

- `agent-android/build.sh` needs the Android SDK (build-tools + platform 35) and JDK 17. It produces the agent
  APK (embedded in the binary at build time, so run it before `cargo`) and the demo APK.
- `ios-runner/build.sh` needs Xcode 16+. It produces the XCUITest runner and the demo app.
- `cargo install` installs to `~/.cargo/bin`; `cargo build --release` on its own puts it in
  `target/release/fastpilot`.
- How to release: [docs/releasing.md](releasing.md).

## MCP configuration

**Claude Code** (from the root of the project you want to test):

```bash
claude mcp add fastpilot -e FASTPILOT_VARS=DEMO_PASS -e DEMO_PASS=rahasia123 -- /path/to/fastpilot mcp
```

or `.mcp.json`:

```json
{
  "mcpServers": {
    "fastpilot": {
      "command": "/path/to/fastpilot",
      "args": ["mcp"],
      "env": { "FASTPILOT_VARS": "DEMO_PASS", "DEMO_PASS": "rahasia123" }
    }
  }
}
```

**Claude Desktop** (`~/Library/Application Support/Claude/claude_desktop_config.json` on macOS,
`%APPDATA%\Claude\claude_desktop_config.json` on Windows): the same `mcpServers` block. Claude Desktop does not
start the server from your project folder, so use absolute paths in `target`/`openUrl`.

Every env below (except `FASTPILOT_STATE_KEY` and `FASTPILOT_LOG`) can also go in
`~/.fastpilot/config.toml`; see [Settings](#settings). Env always wins.

| Env | Purpose |
|---|---|
| `FASTPILOT_VARS` | a comma-separated list of variables a flow may use through `${NAMA}`; their values are always masked as `***` in output |
| `FASTPILOT_CHROME` | path to the Chrome/Chromium executable |
| `FASTPILOT_HEADFUL=1` | show the browser window (same as `fastpilot mcp --headful`) |
| `FASTPILOT_ANDROID_SERIAL` / `FASTPILOT_IOS_UDID` | default device for `session: { start, platform }` in a flow |
| `FASTPILOT_IOS_DEVICE` | the simulator to boot if none is running (default `iPhone 16`) |
| `FASTPILOT_IOS_RUNNER` | the iOS runner build folder (default `ios-runner/build`) |
| `FASTPILOT_DIR` | location of `.fastpilot/` (runs + memory) |
| `FASTPILOT_MEMORY` | `rw` (default), `readonly` (CI), or `off` |
| `FASTPILOT_ALLOWED_DOMAINS` | a comma-separated list of hosts (`*.example.com` for subdomains) that web sessions may open: `openUrl`, links, redirects, and popups to other hosts are blocked (`blocked`). Other resources (CDN, API) still load |
| `FASTPILOT_CONFIRM_ACTIONS` | `default` (hapus, delete, bayar, pay, beli, checkout, transfer, ...) or your own word list: a tap on an element with a name like that is refused (`blocked`) unless the step has `confirm: true` (CLI: `--confirm`). Not set: hapus, delete, remove. An empty env does not turn off the built-in list; you can clear it only with `fastpilot config set confirm_actions ""` or the Clear button on the Limits page |
| `FASTPILOT_BLOCK_ACTIONS` | a list of words that are always blocked. Not set: the built-in list (payments and deleting an account). An empty env does not turn off the built-in list; you can clear it only with `fastpilot config set block_actions ""` or the Clear button on the Limits page |
| `FASTPILOT_STATE_KEY` / `FASTPILOT_STATE_EXPIRE_DAYS` | AES-256 key (64 hex) to encrypt saved logins / their maximum age (default 30 days) |
| `FASTPILOT_FLOW_TIMEOUT` | time limit for one `run_flow`, in seconds (default 300) |
| `FASTPILOT_MODEL` / `FASTPILOT_MODEL_REPLAY` / `FASTPILOT_MODEL_EXPLORE` | models for `fastpilot run` (see below) |
| `FASTPILOT_CONFIG` | location of the settings file (default `~/.fastpilot/config.toml`) |
| `FASTPILOT_ADB_SERVER` | an adb server on another computer, `host:port` (see [Devices on another computer](#devices-on-another-computer)) |
| `FASTPILOT_CHROME_ENDPOINT` | Chrome on another computer for all web sessions: `http://host:port` or `ws://...` |
| `FASTPILOT_LOG` | tracing filter, e.g. `debug` (logs go to stderr; stdout is used by MCP) |

## Tools (25)

The 14 below are for driving a test, plus `related_tests` and the Jira tools. Eleven more do what the dashboard pages
do: `apps`, `tests`, `schedules`, `map`, `ship_check`, `secrets`, `memory`, `history`, `reports`, `agents` and `providers`.
No tool takes a password's value or an API key.

| Tool | Purpose |
|---|---|
| `session` | `start` / `stop` / `list` / `devices` / `start_device`. `start` returns the first snapshot, and a path memory hint if a `test_id` is given. Web: `mode: launch\|attach` (+`endpoint`). Android/iOS: `endpoint` = serial/UDID (for `start_device`: AVD/simulator name) |
| `run_flow` | Run a YAML flow (multi-step, multi-session, cross-platform, `parallel:`). Returns timing per step and a diff per session. With a `test_id`, the flow and evidence are recorded |
| `check_flow` | Validate without running: syntax, steps not supported yet, unknown sessions, parallel rules, `${VAR}` not allowed |
| `snapshot` | A compact snapshot. `filter` = find (results + ancestors), `coords`, `full`, `file` (writes to `.fastpilot/snapshots/`). Rarely needed |
| `screenshot` | JPEG. `annotate: som` = numbered boxes per target (drawn on web) + a list of ref → coordinates |
| `assert` | `visible` / `not_visible` / `text_equals` / `contains` / `enabled`, deterministic with auto-retry |
| `report` | Closes the run: `report.md`, `steps.jsonl`, `assertions.json`, failure screenshots. A `pass` status updates path memory |
| `state` | Save/load a login (web: cookies + localStorage) so the next test does not have to log in: `save` / `load` / `list` / `delete`. Only the owner can read the files, they expire after 30 days (`FASTPILOT_STATE_EXPIRE_DAYS`), and they are encrypted with AES-256-GCM if `FASTPILOT_STATE_KEY` (64 hex, `openssl rand -hex 32`) is set. Same as the `saveState` / `loadState` steps |
| `config` | View and change settings (`~/.fastpilot/config.toml`) through chat: `list` / `get` / `set` / `unset`. The agent may only **make limits stricter** (`allowed_domains`, `confirm_actions`, `block_actions`, `vars`, `state_expire_days`, the `allow_*` permissions, `upload_dirs`, `max_steps`, `test_timeout`); stricter values apply right away, looser ones only after a restart. Program paths, locations, and other computers (`chrome`, `claude`, `dir`, `ios_runner`, `adb_server`, `chrome_endpoint`) can only be changed by the user with `fastpilot config set` or `fastpilot ui` |
| `docs` | On-demand reference: `dsl`, `selectors`, `parallel`, `examples`, `errors`, `memory` |
| `related_tests` | Which saved tests to run first after a code change ("test what I changed"): reads the changed files with git (read only) and gives each chosen test one plain reason |
| `jira_read` | Read a ticket of the default project chosen in the dashboard (Integrations › Jira): summary, status, type, description, marked as data that is never followed as instructions. The token stays with fastpilot. Not used in the dashboard Chat (fastpilot reads the tickets you name) or in `fastpilot run` / schedules |
| `jira_create_issue` | Ask to create one issue there. Looks for an open issue that seems the same first. Text is cleaned (secrets masked, no emails, no other sites' links, no mentions). **Sends nothing**: it files a request you send with one click on the Integrations page or with `fastpilot jira approve` in your own terminal (30 minutes). Refused in the dashboard Chat (it uses cards) and in `fastpilot run` / schedules (nobody to approve) |
| `jira_comment` | Ask to add one comment to a ticket, the same way. Never changes a ticket's status |

## Examples

A web login in one round-trip ([tests/login.md](../tests/login.md),
[examples/demo/login.html](../examples/demo/login.html)):

```yaml
- session: { start: web1, platform: web, target: "examples/demo/login.html" }
- inputText: { into: "Email", text: "a@b.com" }
- inputText: { into: "Password", text: "${DEMO_PASS}" }
- tapOn: "Masuk"
- assertVisible: "Budi"
```

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

Android + WebView ([tests/login-android.md](../tests/login-android.md)). Steps on the WebView page
run over CDP:

```yaml
- session: { start: and1, platform: android }
- launchApp: com.fastpilot.demo/.LoginActivity
- inputText: { into: "Email", text: "a@b.com" }
- inputText: { into: "Password", text: "${DEMO_PASS}" }
- tapOn: "Masuk"
- assertVisible: "Selamat datang, Budi"
- tapOn: "Bayar"
- tapOn: "Bayar sekarang"
- assertVisible: "Pembayaran berhasil"
```

Opening the app against a staging server or in debug mode without rebuilding it (`launchApp` with `arguments` and `env`):

```yaml
- launchApp:
    appId: com.example.app
    arguments: { server: "https://staging.example.com", debug: true, retries: 3, features: [new, beta] }
    env: { API_TOKEN: "${STAGING_TOKEN}" }   # iOS only
```

- `arguments`: on Android these become intent extras (text, whole numbers, `true`/`false`, lists of text without
  commas; read them with `getIntent().getStringExtra(...)` and similar). On iOS they become launch arguments
  `-server https://...` (read them with `UserDefaults.standard.string(forKey: "server")`).
- `env` (iOS): the app's environment (`ProcessInfo.processInfo.environment`). Its values always show as `***`
  in output and reports; a recorded run only stores the `${NAMA}` reference, so put secrets in
  `FASTPILOT_VARS`. Names that change how the app is loaded (`DYLD_*`, `LD_*`, `MALLOC*`, `OBJC_*`,
  `XPC_*`) and `FASTPILOT_*` are refused.

Cross-platform in one `run_flow` ([tests/login-cross-platform.md](../tests/login-cross-platform.md)):

```yaml
- session: { start: web1, target: "examples/demo/login.html" }
- session: { start: and1, platform: android }
- session: and1
- launchApp: com.fastpilot.demo/.LoginActivity
- parallel:
    - session: web1
      steps:
        - inputText: { into: "Email", text: "a@b.com" }
        - inputText: { into: "Password", text: "${DEMO_PASS}" }
        - tapOn: "Masuk"
        - assertVisible: "Budi"
    - session: and1
      steps:
        - inputText: { into: "Email", text: "a@b.com" }
        - inputText: { into: "Password", text: "${DEMO_PASS}" }
        - tapOn: "Masuk"
        - assertVisible: "Selamat datang, Budi"
```

A failed step stops early and returns candidates:

```
fail step 1/1 [web1] tapOn "Hapus Akun": not_found: no element matches "Hapus Akun" (timeout 5000ms)
candidates: e10 button "Keluar"
```

Path memory: `session start` with a `test_id` returns a YAML hint from the last passing run. Send that hint
as one `run_flow` with the same `test_id`, then close with `report`. See the `docs` topic `memory`.

## Shell CLI: one command per step

For agents that can only run shell commands, or for trying things by hand. Sessions stay open in a
background daemon (`~/.fastpilot/cli.sock`, started automatically by the first command; it stops by itself
after 30 minutes without commands, or with `fastpilot daemon stop`), so each command is fast.

```bash
export FASTPILOT_VARS=DEMO_PASS DEMO_PASS=rahasia123   # read by the daemon when it starts
fastpilot open examples/demo/login.html
fastpilot snapshot -i                  # only elements you can act on, with refs
fastpilot fill @e2 a@b.com
fastpilot fill "Password" '${DEMO_PASS}'
fastpilot click @e5
fastpilot wait Budi && fastpilot get title
fastpilot screenshot result.jpg --annotate
fastpilot close
```

- Selectors: `@e7` (a ref from a snapshot), `"Sign in"` (text), `#email` (id), `role=button:Sign in`, `120,340` (a point).
- Commands: `open`, `attach`, `close [--all]`, `sessions`, `devices`, `snapshot [-i] [--json] [--full]
  [--coords] [--filter T]`, `click`/`tap`, `dblclick`, `longpress`, `fill`, `type`, `clear`, `press`,
  `select`, `check`/`uncheck`, `upload`, `scroll`, `scrollto`, `swipe`, `back`, `forward`, `reload`,
  `wait <ms|sel> [--gone]`, `wait --url`, `assert`, `is visible|enabled`, `get text|url|title`, `eval`,
  `screenshot`, `record start|stop`, `set viewport|locale|geo|network|permission`, `launch`, `stop-app`,
  `clear-state`, `hide-keyboard`, `flow <file|->`, `check-flow`, `state save|load|list|delete`,
  `diff screenshot <name> [--update] [--ignore sel]`, `errors`, `ship-check [url]... [--links N]`,
  `docs`, `daemon status|stop`.
  `fastpilot help` shows them all.
- Web, Android and iOS work the same way: `fastpilot open com.app/.MainActivity --platform android`.
- Several sessions at once: `--session NAME` (or `FASTPILOT_SESSION`); `--json` for raw output.
- Exit code ≠ 0 if a step fails; the output is the reason, the candidates, and the current screen, the same as MCP.
- macOS and Linux (Unix socket). Secrets in `FASTPILOT_VARS` are still masked as `***`.

## Ship check: one verdict for the pages before a release

```bash
fastpilot ship-check https://staging.shop.example https://staging.shop.example/cart [--links N] [--cookies]
```

For each page, fastpilot opens it in Chrome and checks six things:

- **Console errors**: exceptions and `console.error`. An uncaught error means the page's own script crashed.
- **Failed requests**: HTTP errors, requests that did not finish, and the page's own status.
- **Broken links**: up to 20 links to the same site (`--links N`, 0 for none), asked with `HEAD`.
  - Five at a time, and without your cookies, so they are asked signed out. A link that needs sign-in
    is "not checked", not broken.
  - Many servers treat `HEAD` like `GET`, so `--cookies` (`{ cookies: true }`) asks signed in only on
    request. It then also skips links with a query string or a word for a change (clear, archive,
    accept, ban, reset, ...).
  - Redirects are not followed.
  - Links to other sites are counted, not opened.
  - Links whose text or address names a risky action are never requested: the block and confirm
    lists, sign out, delete, unsubscribe.
  - The page only lists its links; fastpilot decides which ones are asked.
  - Text from the page (names, errors, addresses) is shown on one line and shortened.
- **Layout at 3 sizes**: phone 390, tablet 768 and desktop 1280 px. It looks for content wider than the
  screen, or a page without a mobile viewport. The size is put back afterwards.
- **Accessibility basics**: page language and title, images without `alt`, fields, buttons and links
  without a name, and repeated ids.
- **Speed**, measured last, from Chrome itself (DevTools events; nothing is added to the page):
  - **Loads in** (load time), **Main content shown** (LCP), **Layout shift** (CLS), requests and
    their size, and page errors. Always at 1280 × 800 on a normal connection.
  - A value Chrome does not give is "not measured", never estimated.
  - The page is opened once more with a plain `GET`. A page that came from a form, or whose address
    (or the address it ended on after a redirect) carries a one-time code or key (a parameter such as
    `token`, `access_token`, `api_key`, `code`, `session`, `..._id`, also after `#`, or a long random
    part in the path) or a word for a change (delete, sign out, unsubscribe, confirm, ...) is not opened
    again: Speed then says "Not measured" and why.
  - Each result is kept on this computer, not in the project:
    `~/.fastpilot/projects/<project>/ship/<host>.json` (owner-only, the last 100). Passwords in
    addresses, test variables, and the values of code or key parameters are masked; in page errors and
    failed requests every parameter value is. It is compared
    with the previous check of the same page. Flagged: more than 20% **and** more than 0.3 s slower,
    layout shift up by more than 0.1, a new page error or a new failed request. Within 10% is "no change";
    more than twice as slow is "much slower (0.2 s → 3.4 s)" and a quarter of the time or less is
    "much faster", never a huge percentage. LCP and CLS appear only in the Details line and in a
    tooltip on the dashboard's release card.
  - Speed alone gives at most SHIP WITH FIXES, never DO NOT SHIP, and "not measured" does not change
    the verdict.

It never clicks, types or submits.

The verdict follows the `release-check` skill:

| Verdict | When |
|---|---|
| DO NOT SHIP | The page did not load, or its own script crashed |
| SHIP WITH FIXES | Only other problems |
| INCONCLUSIVE | A check could not run, or was not asked |
| SHIP | Nothing missing and nothing wrong |

Missing evidence is never a pass. The report always names what was not checked: colour contrast,
keyboard use, the look of pages, anything behind sign-in. It also names the regression and the main
journeys, which this command does not run.

Exit codes: 0 for SHIP or SHIP WITH FIXES, 1 for DO NOT SHIP, 2 for INCONCLUSIVE.

In a test or from an AI, the same check is the `shipCheck` step: `- shipCheck` or `- shipCheck: { links: 5, cookies: true }`.

In the dashboard, ask the Chat bot "Can we release?". It runs the regression, the main journeys and
`shipCheck`, then shows a **release verdict card**. The card has the verdict, why, the evidence with
checked / problem / could not run / not checked, what to fix, and an always-visible "Not checked" box.
A bug report it writes shows as a **preview card**: it is not sent anywhere, and you can copy it as
text or Markdown (page and bot text stays plain text when pasted). The verdict card is labelled "the bot's
summary": the bot wrote it from the checks it ran, and fastpilot did not verify it.

## Without opening Claude Code: `fastpilot run`

Runs natural-language test cases (`tests/*.md`) without a chat session: fastpilot calls **the user's own
Claude Code** (`claude -p`, with the user's own login and subscription) and gives it the fastpilot tools through MCP.
fastpilot does not call a model API itself, so **you do not need an API key**. You need `claude` installed
and signed in once (`claude`). For another location: `fastpilot config set claude /path/ke/claude`.

```bash
DEMO_PASS=rahasia123 FASTPILOT_VARS=DEMO_PASS \
  fastpilot run tests/ --junit junit.xml --html report.html
```

**Only what you changed** (`--changed [BASE]`): fastpilot reads the files changed since BASE (default:
where your branch left `main`; uncommitted and new files count too) and runs the 5 most related tests
first, each with one reason in plain words ("chosen because: you changed the login page"). The others
are listed as **Not run (N)** and never counted as passed; add `--all-after` to run them after the related
ones. Without git, or with no changes, it says so and runs every test. In Chat, ask "test what I changed".

```bash
fastpilot run tests/ --changed            # compared with main
fastpilot run tests/ --changed release-1  # compared with a branch, tag or commit
```

- Claude Code gets only the fastpilot tools: its built-in tools (shell, files, web) are turned off and the fastpilot
  tool permissions are approved automatically, because nobody is waiting in front of the screen. The `report` and `config` tools
  are not given.
- Tests with a path memory hint are replayed by a fast model (`claude-haiku-5-5`). If the replay fails
  or there is no hint yet, a balanced model (`claude-sonnet-5-5`) continues from the last screen (both can be
  changed per agent on its Model tab, for all agents under Settings › AI model › Default models, or with `model_replay`,
  `model_explore`; Opus is a choice there, not the default). `--model` (or `model`) forces one model, and `--max-steps` limits the number of tool calls per test (default 30).
- Failures are retried while `FASTPILOT_RETRY_BUDGET` (default 3) is not used up; a retry that passes = flaky.
  After `FASTPILOT_BREAKER` (default 3) infra failures in a row, the remaining tests are skipped as infra.
- `--workers N` runs N tests at once. Each test has its own Chrome. Booted Android devices and iOS simulators
  are lent to one test at a time (checked for health when lent; a device that
  fails the check is taken out), a retry uses another device if there is one, and the queue is ordered by the longest
  recorded duration (tests that have never run go first).
  With `FASTPILOT_ANDROID_SERIAL` (or `ANDROID_SERIAL`) set, only that Android device is used and no other one is contacted; without it, every connected device is. The first line of the run says which.
- **In GitHub Actions:** [`autobot-lab/fastpilot-action@v1`](https://github.com/autobot-lab/fastpilot-action)
  installs the release for the runner (SHA-256 checked) and runs `fastpilot run <tests> --junit … --html …`;
  its README has a copy-paste workflow with the JUnit report on the pull request and the HTML report as an artifact.
- `--record` records the screen of each session (web: a CDP screencast as an HTML filmstrip; Android: `screenrecord`;
  iOS: `simctl io recordVideo`). `--html` produces one self-contained HTML file with the report for each
  run (steps, durations, diffs, screenshots, recordings); `--junit` is for CI dashboards.
  In each run's report, the recording has a marker for each step (red on failed steps; click to jump there)
  and one button per flow that plays only that flow's part. A flow that passes includes a screenshot
  of its final screen.
- `--reset` makes each test start from a clean state: Android/iOS sessions reset their app before it opens,
  once per app per run (per test for `run`, per process for `mcp`): through the session's `target`, or on the
  first `launchApp` for that app
  (Android: `pm clear` for data and runtime permissions, portrait screen; iOS simulator: data and all privacy permissions,
  orientation cannot be set through simctl; physical iPhones are not supported). Web sessions always start with
  a new browser context anyway (no cookies or storage). Needs `allow_app_reset` (built-in: allowed); if it is turned off,
  mobile sessions are refused with `blocked`. `fastpilot mcp --reset` does the same for sessions from chat.
  Orientation and device settings are restored when the session ends.
- `--report-github` (in GitHub Actions, on a pull request) writes the suite results as **one** comment on the PR:
  "running" when it starts, then replaced by a results table (passed, failed + class + notes, flaky, quarantined,
  duration, link to the workflow run). The same comment is updated on later runs (marked
  `<!-- fastpilot-report -->`), so comments do not pile up. The token is read only from the `GITHUB_TOKEN` env; it is never
  stored, put into a URL, or printed. A failure to write the comment does not fail the suite.
  `--report-github=dry` prints the comment content without sending anything. Example job:

  ```yaml
  permissions:
    pull-requests: write
  steps:
    - run: fastpilot run tests --report-github
      env:
        GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
  ```
- **Besides Claude Code (optional):** `fastpilot run` can be driven by an OpenAI-compatible API
  (chat completions + tool calling). Claude Code stays the built-in default. The tools, the `finish` verdict, `max_steps`, and
  `test_timeout` are exactly the same.

  ```bash
  fastpilot config set provider openrouter        # openai, openrouter, groq, ollama (local), custom
  fastpilot config set provider_model <model id>  # required
  fastpilot config set-key openrouter             # key is read from stdin (hidden) and stored in Keychain
  fastpilot config test-provider                  # one small request to check
  ```

  The key is stored in the macOS Keychain (Linux: the secret service through `secret-tool`; if that is missing: a 600 file in
  `~/.fastpilot/keys/`), or given through the `FASTPILOT_PROVIDER_API_KEY` env. The key is never written to
  config, logs, reports, or process arguments. Only the user can change `provider` and `provider_base_url` (an agent
  cannot redirect test data to another server). `http://` is only for this computer (e.g. Ollama); other servers
  must use `https://`. To go back to Claude Code: `fastpilot config unset provider`.

  A key is tied to the server it was stored for: presets (`openai`, `openrouter`, `groq`) only go to their own
  server, and `custom` goes to the `provider_base_url` set when `set-key` was run. If the server is different, the key is not
  sent and fastpilot tells you what to do (set the key again, or restore the URL). Keys stored by an
  older fastpilot version (before keys were tied to a server) are only used for hosted preset servers; for `custom` or
  `ollama`, run `fastpilot config set-key <provider>` once more. Redirects from the provider's server
  are not followed (the request content does not go to another server). Changing `provider`
  with `fastpilot config set provider …` deletes the old `provider_base_url`, so the new provider uses
  its preset. `FASTPILOT_KEYSTORE=file` forces storage in a 600 file (useful in CI, or with an isolated `HOME`
  so macOS does not ask for Keychain).

  **Privacy:** with a third-party provider, the tool content sent to the model (screen snapshots, step results,
  page URLs, including local paths like `file:///Users/<nama>/…`) also reaches that provider. `${NAMA}`
  values are still masked as `***`. Use a local Ollama if test data must not leave this computer.

  Since 0.7.2 this is also on a page: `fastpilot ui` › Settings › AI model › **Providers** (**+ Add a provider**: OpenAI,
  OpenRouter, Groq, Ollama or Other with its address; the key is sent once to the key store and never shown again; the
  list of models is loaded from the provider, and a model id you type still works) and **Default models**. Each agent can
  choose its own Replay and Explore-and-repair model on its **Model** tab (its Chat answers with the Explore-and-repair
  model, so Chat messages go to that provider too), or in a terminal with
  `fastpilot agent model NAME --replay X --explore Y` and `fastpilot provider list|add|remove|refresh|models`. We have tried
  Claude only, and have not run real OpenAI, OpenRouter or Groq accounts.
- Exit code ≠ 0 if a test outside quarantine fails. `FASTPILOT_VARS` values written in a
  test case are sent to the model as `${NAMA}`, never as their values.

## Everyday Chrome and physical iPhones

- **Your everyday Chrome without restarting it**: load `extension-chrome/` (chrome://extensions → Developer mode →
  Load unpacked), request `session { action: start, mode: attach, endpoint: extension }`, then click the
  extension button in the tab you want to control. The relay only accepts connections that carry a random per-session token;
  the extension gets it through native messaging (web pages cannot reach it). Details in
  [extension-chrome/README.md](../extension-chrome/README.md).
- **Physical iPhone/iPad and Android phones**: see [docs/real-devices.md](real-devices.md). In short:
  check readiness with `fastpilot ios doctor`, build the runner for iPhone, then start a session with
  `device: ios-device:<udid>` or `device: android-device:<serial>`.

```bash
FASTPILOT_IOS_TEAM=<team id> FASTPILOT_IOS_BUNDLE_PREFIX=com.<nama>.fastpilot ios-runner/build.sh --device <udid>
```

## Devices on another computer

The Android emulator and Chrome can run on another computer (e.g. a Linux PC with KVM), while fastpilot
and the AI agent stay on your laptop. All traffic goes through an **SSH tunnel**; debug ports are never opened to the
network.

> The adb port (5037) and Chrome's remote debugging port give full control over the device and the browser,
> with no password. Never open them to the internet or the LAN (`adb -a`, `--remote-debugging-address=0.0.0.0`).
> Keep both listening on `127.0.0.1` on that computer and connect through SSH.

**Android.** On the device computer: start the emulator/phone as usual (`adb devices` lists it). On your laptop:

```bash
ssh -N -L 15037:127.0.0.1:5037 user@device-computer
```

```bash
fastpilot config set adb_server 127.0.0.1:15037      # or the env FASTPILOT_ADB_SERVER
fastpilot doctor android                             # adb server 127.0.0.1:15037 ✓, device ✓
```

- Use a local port other than 5037, so it is not mixed up with the adb server on your own laptop.
- fastpilot passes its connections to the agent and the WebView through that adb server itself, so one tunnel is enough.
- Things that only work from the emulator's computer: `setLocation`, `setNetwork` (throttling) and other emulator console
  commands (refused with a clear message), and `session start_device` (starting an AVD).

**Chrome.** On that computer, run Chrome with a separate profile and the debug port on localhost only:

```bash
google-chrome --headless=new --remote-debugging-port=9222 --user-data-dir=/tmp/fastpilot-chrome
```

On your laptop:

```bash
ssh -N -L 19222:127.0.0.1:9222 user@device-computer
```

```bash
fastpilot config set chrome_endpoint http://127.0.0.1:19222   # or the env FASTPILOT_CHROME_ENDPOINT
fastpilot doctor web                                          # remote Chrome at http://127.0.0.1:19222 ✓
```

- Each session still gets its own `BrowserContext` (separate cookies and storage); fastpilot does not
  start a local Chrome.
- File paths are read by the Chrome computer: a local `target` (`examples/demo/login.html`), `uploadFile`, and
  downloads use that computer's file system. Use an `http(s)://` URL for the app you test.
- Only the user can change `adb_server` and `chrome_endpoint` (`fastpilot config set`), not the agent.
- If the host is not this computer (not `127.0.0.1`/`localhost`), `doctor` and the logs show a warning: DevTools
  and adb have neither a password nor encryption. `user:password@` in the address is masked (`***@`) in `doctor` and error messages.

Running **fastpilot itself** on another computer (e.g. a Mac for iOS) over SSH, opening `fastpilot ui`
from your laptop, and saving iOS simulator memory with simslim: see [docs/remote-machine.md](remote-machine.md).

## Bench

```bash
./target/release/fastpilot bench --iterations 30
```

```bash
./target/release/fastpilot bench android --iterations 20
```

Results on an M1 Pro (headless Chrome; Android emulator API 35; iPhone 16 simulator, iOS 18.1):

| Metric | Web | Android | iOS | PRD target |
|---|---|---|---|---|
| Session start | 149 ms (warm) | 629 ms | about 23 s (xcodebuild starts the runner) | <1 s / <2 s |
| Snapshot p50 | 1.1 ms | 4.3 ms | 16–55 ms | <50 / <100 ms |
| Tap → diff p50 | 119 ms | 154 ms (with tap verification) | about 0.4–1.4 s (XCUITest waits for animations) | <200 ms |

iOS was measured by hand with a protocol client; there is no `bench ios` yet.

Token cost of what the AI reads (offline approximation; the rules are printed in the output):

```bash
./target/release/fastpilot bench tokens web      # or android / ios
```

| What the AI reads (approximation) | Web | Android |
|---|---|---|
| Login screen snapshot | 111 | 86 |
| `run_flow`, 1 step (tap → diff) | 92 | 60 |
| `run_flow`, 4-step login | 236 | 379 |
| Screenshot, for comparison (w·h/750) | 1366 | 1229 |

Stability (PRD G6) is measured with pass^k and an upper bound on the failure rate, not with a single "passed 20 times" number:

```bash
FASTPILOT_VARS=DEMO_PASS DEMO_PASS=rahasia123 ./target/release/fastpilot stability examples/flows/login-web.yaml --runs 30
```

```
30/30 passed (100.0%), p50 1.11s
pass^1: 1.000
pass^3: 1.000
pass^8: 1.000
failure rate <= 11.4% (95% Wilson upper bound); a <=1% claim needs >= 381 runs without failure
```

## Test

```bash
cargo test
```

- Unit tests: `crates/core` (DSL, `check_flow`, prune/ref/diff, selector resolution, engine), `crates/memory`,
  and node conversion in `crates/android` and `crates/ios`.
- `bin/fastpilot/tests/mcp_login.rs` runs `fastpilot mcp` over stdio with a real Chrome:
  - login, stop-early, two parallel sessions;
  - widgets (select, clicking a div, confirm dialog, pressKey, upload, iframe, shadow DOM);
  - assert, report, and path memory;
  - attaching to a Chrome that is already running.
- `bin/fastpilot/tests/web_advanced.rs` tests cross-site iframes (OOPIF), popups, downloads, `opacity:0`
  elements, console/HTTP errors in failure reports, Set-of-Mark, snapshot find/file, and tools in other sessions
  that do not wait for a running flow.
- `bin/fastpilot/tests/android.rs` tests Android login, WebView, and a parallel web+Android scenario on the
  emulator named in `FASTPILOT_ANDROID_SERIAL`.
- `bin/fastpilot/tests/ios.rs` tests login on the iOS simulator.
- `bin/fastpilot/tests/cli.rs`, `cli_shell.rs`, `config_web.rs`, `state_web.rs`, `policy_web.rs` and
  others test the CLI, the shell daemon, settings, saved logins, and guard rails in a real Chrome.
- Android tests (`android.rs`, `ui_takeover_device.rs`, the Android demo in `demo_cli.rs`) are never
  skipped silently and never pick a device by themselves:
  - run them with `FASTPILOT_ANDROID_SERIAL=emulator-5556` (an emulator that tests may use);
  - without it, without a device, with a device that is not ready, without adb, or with APKs not built yet
    (`agent-android/build.sh`): the tests fail and name the cause;
  - `FASTPILOT_SKIP_ANDROID=1` skips them on purpose, and the output says so.
- Tests that need Chrome or an iOS simulator are skipped automatically if those are not available.
- GitHub CI only builds (`cargo build --workspace --all-targets --locked`); tests are run locally.

## Layout

```
crates/core      config (env → config.toml), policy, Driver trait, Selector, Snapshot (prune, stable ref, diff), DSL + check_flow, engine, assert, SessionRegistry
crates/web       CDP client, launch/attach Chrome, WebPage (Driver) per BrowserContext
crates/android   adb server protocol, agent client, AndroidDriver (+ WebView via CDP)
crates/ios       simctl/devicectl + XCUITest runner, usbmux, Web Inspector, IosDriver
crates/memory    run evidence (.fastpilot/runs) + path memory (.fastpilot/memory)
crates/mcp       rmcp stdio server, 14 tools, docs
bin/fastpilot    entrypoint: `mcp`, shell CLI + daemon, `run`, `doctor`/`setup`, `config`, `bench`, `stability`
agent-android/   instrumentation APK (Java, UiAutomation), built without Gradle
ios-runner/      XCUITest runner + iOS demo app
examples/        web demo (login, widgets), Android demo
tests/           natural-language test cases
skills/fastpilot Agent Skill (SKILL.md) for AI agents
```

## Known limitations

- Physical iOS devices: the code exists (usbmux/tunnel, `fastpilot ios doctor`) but has not been tested on hardware.
  You need an Xcode that matches the iOS version (iOS 26 → Xcode 26) and an Apple ID in Xcode; a free profile lasts
  7 days and allows at most 3 apps per device. `setLocation`, `pushNotification`, screen recording, `clearKeychain`
  and allow/deny permissions only exist on the simulator. See [docs/real-devices.md](real-devices.md).
- iOS WKWebView: through Web Inspector, only if the app sets `webView.isInspectable = true` (iOS 16.4+);
  on an iPhone, also Settings → Safari → Advanced → Web Inspector. A web tap is a DOM `click()` (not a
  touch), with several WKWebViews the first page is used, and the RemoteXPC path does not exist yet.
- iOS `setNetwork`: not supported (the simulator uses the Mac's network).
- Web attach through the extension relay ([extension-chrome/](../extension-chrome/)) needs a click on the extension button
  in the tab you want to control; without the extension: Chrome M144+ `autoConnect` or `--remote-debugging-port`.
