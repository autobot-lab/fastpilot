## What's new in 0.6.0

Connect Jira, see which screens have a test, compare one test across web, Android and iPhone, run only the
tests your change touches, and find out why a test is unstable.

### Apps and Jira
- **Apps › Connect Jira** in the dashboard: your Jira address, Atlassian email and an API token, then
  **Test connection**, choose the default project, and **Save**. The token goes to the Keychain and is never
  shown again or given to the AI, Chat, logs or reports. **Manage** has Replace token and Disconnect.
- The dashboard only connects. Issues and comments come from the **Chat bot** or **Claude Code**, always as a
  draft you approve:
  - **In Chat**, ask in your own words ("make a Jira ticket for the voucher bug", "add the result to
    KAN-12"). A card shows the site, the project or ticket and the exact text. Nothing is sent before you
    click **Create issue** or **Add comment**. A short "yes" to a draft the bot offered gives the same card.
  - **From Claude Code**, nothing is sent by the AI: each issue or comment waits under **Apps › Waiting for
    your OK**, and you send it with one click there or with `fastpilot jira approve` in your own terminal.
  - Name a ticket ("test KAN-12") and fastpilot reads it as information, never as instructions. A ticket,
    page or file can never make a card, and no card changes a ticket's status.

### App map
- **Map** (dashboard, or `fastpilot map`) walks through your app like a visitor, draws every screen it finds,
  and colours each one: **Has a test**, **Test failing** or **No test yet**. Click a screen for **Make a test
  for this screen**.
- It is read-only: on the web it refuses form posts and background saves before they are sent, and it never
  presses Pay, Delete, Sign out and similar words. Every button it skipped is named.

### One test on web, Android and iPhone, side by side
- A test whose `platform:` line names several platforms (or `all`) runs once on each, and the report starts
  with a **Side by side** table: steps as rows, platforms as columns, with the screen after each step. A row
  is marked when a step passes on one platform and fails on another, or when the text it found differs.
- A platform without a device, or one that does not start, says **Not run** with the reason. It is never
  counted as a pass or as a bug in your app.

### Run only what you changed
- `fastpilot run tests/ --changed` (or "test what I changed" in Chat) reads the files you changed with git and
  runs the related tests first, each with one plain reason ("chosen because: you changed the login page").
  New files that git does not track yet count too. Without git, or with no changes, it runs every test.

### Ship check speed
- Ship check now measures page speed from Chrome itself (nothing is added to the page) and compares it with
  the previous check of the same page: "Loads in 2.8 s, up from 2.0 s". Big changes are said in words ("much
  slower"), never as huge percentages. Release watch tells you when a page gets slower.
- Results are kept on this computer, not in the project, and code or key values in addresses are masked.

### Why is this test unstable?
- **Memory › Why unstable?** shows the evidence first (which step, which error, how often it passes on a
  retry), then the likely cause and a suggestion. A test that also passes in some runs is called "flaky, not
  broken"; "a real problem" only when every recent run failed the same way.

### Other changes
- `fastpilot run` no longer hangs when the iPhone helper cannot start: it says "Not run" with the reason
  ("iPhone runner did not start (machine busy?)"), prints progress every 30 seconds, and waits at most
  5 minutes (`FASTPILOT_IOS_START_TIMEOUT`, up to 15).
- Clear message when the Android helper is missing, and every published program is checked to carry it.
- A closed pipe (`fastpilot … | head`) stops quietly instead of crashing.
- The README starts with a 60-second demo.
- The MCP Registry listing now waits until npm shows the new version.

### Known limits
- **Jira Cloud only** (addresses ending in `.atlassian.net`); Jira Server and Data Center are not supported.
  There is no automatic QA queue yet. Jira was tested against a stand-in Jira server, not a real Jira site.
- **App map:** one map per app (the latest is shown). On a phone there is no network guard, only the button
  rules. A plain page load that changes data on the server looks like any other read and still goes through,
  so map a staging or test copy of your app, not production. Map on a phone was not tested with a test
  account.
- **Side by side:** the same text in a different place is not flagged, and the step results come from the
  AI. With two platforms that differ on one step, both columns say "Differs", even if only one is wrong.
- **iPhone:** iPhone sessions started from Claude Code or dashboard Chat still need **Set up iPhone testing**
  (or `fastpilot setup ios`) once. The iOS demo app with the planted label difference must be built from a
  checkout (`ios-runner/build.sh`).
- Chat can say it filed a bug when it only described one in words; the notice under it says correctly that no
  card was made.
- Not tested yet: models other than Claude, Linux, physical Android phones and iPhones.

## What's new in 0.5.1

Small fixes after the 0.5.0 checks.

- **Plugin or `fastpilot init`, not both.** The README says so: together they start two fastpilot
  servers. With the plugin, a secret such as `DEMO_PASS` works like this: add its name in `fastpilot ui` ›
  Settings › Secret, and set its value with `export DEMO_PASS=…` before you start Claude Code. When a
  secret is missing, the message now names the plugin case too.
- **Jira skills explain the connection themselves.** Each skill gives the two steps:
  `claude mcp add --transport http atlassian https://mcp.atlassian.com/v2/mcp`, then `/mcp` › atlassian ›
  sign in, picking the site that has your project. If Atlassian says a site "isn't explicitly granted",
  the skill asks you to sign in again and pick that site, and never switches to another site.
- **Run now works again after its run was stopped from outside.** A scheduled run that was killed no
  longer stays "already running" until `fastpilot ui` restarts.
- Settings › Limits & permissions no longer shows the old "New in 0.3.2" notice.
- The skill count is the same everywhere: the Skills page says "23 on, plus the fastpilot skill (always
  on)", 24 in all, as in the README.

### Known limits
- The Jira skills have not been tested against a real Jira site yet.
- The known limits of 0.5.0 below still apply.

## What's new in 0.5.0

fastpilot where you already work: a Claude Code plugin, the official MCP Registry, GitHub Actions and
Jira.

### Claude Code plugin
- Install fastpilot in Claude Code in one step: `/plugin marketplace add autobot-lab/fastpilot`, then
  `/plugin install fastpilot@fastpilot`. You get the fastpilot tools and its testing skills, shown as
  `fastpilot:<skill>`, for example `fastpilot:boundary-value-analysis`.
- Use either the plugin or `fastpilot init` in a project, not both: with both, Claude Code shows two
  fastpilot servers.

### Official MCP Registry
- fastpilot is listed on the official MCP Registry as `io.github.autobot-lab/fastpilot`, so MCP clients
  that read the registry can find and install it.

### GitHub Actions
- Run your fastpilot web tests in CI with
  [`autobot-lab/fastpilot-action@v1`](https://github.com/autobot-lab/fastpilot-action). It downloads
  the fastpilot release for the runner, checks its SHA-256, runs your tests, and gives a JUnit file for
  the checks tab and an HTML report. Pull requests from forks get no secrets, so their tests show
  "could not run" instead of a pass.

### Jira
- **Three Jira skills**, used through Atlassian's own MCP server in Claude Code:
  - **Jira ticket to test** reads a ticket's acceptance criteria and turns them into tests. It never
    writes to Jira.
  - **Jira bug report** turns a failed run into one new Jira issue (steps, expected and actual result,
    links to the report and video).
  - **Jira test result** posts one comment with the result. It changes the ticket's status only when you
    ask for it in your own words.
- Every write shows the exact draft, including the Jira site, and waits for your "yes". With nobody to
  ask (a scheduled or unattended run) nothing is written. Ticket and page text is treated as data, never
  as instructions, and emails, names and order numbers from tested pages are masked.
- Setup: connect Atlassian's MCP server in Claude Code (`/mcp` to sign in) and keep Claude Code's own
  question on for every Jira write. See the guide's "Jira" section. Jira works in Claude Code, not in
  dashboard Chat.

### Docs
- The README shows the dashboard: Chat creating a test, the Device page, Live and Schedules.

### Known limits
- **The Jira skills have not been tested against a real Jira site yet**, only with a stand-in Atlassian
  server. The Atlassian endpoint in the guide was checked through Claude's built-in Atlassian connector,
  not against Atlassian's own documentation.
- With the plugin alone, a test that needs a secret such as `${DEMO_PASS}` stops and asks you to set it:
  put it in the project's `.mcp.json` (as `fastpilot init` and `fastpilot demo` do) or export it.
- The Jira skills point to the guide for setup; npm and plugin installs have no local copy of the guide,
  so use the [online guide](https://github.com/autobot-lab/fastpilot/blob/main/docs/guide.md#jira).
- Settings › Limits & permissions still says "New in 0.3.2" about blocked payment buttons.
- Not tested yet: models other than Claude, Linux, physical Android phones and iPhones.

## What's new in 0.4.2

Installs on npm 12, says plainly when a test could not run, and sets up iPhone testing by itself.

### Install
- **Works on npm 12.** npm 12 skips install scripts unless you allow them, so 0.4.1 installed without
  its program and `fastpilot` failed to start. Now the first run downloads the program for your
  computer, checks its SHA-256 against the GitHub Release, and then starts. This happens once; later
  runs need no network. Messages go to stderr, so Claude Code's connection to fastpilot is not
  disturbed. npm 11 and older still download it at install time.
- The download stops after 2 minutes or 150 MB, and a mirror set with `FASTPILOT_DOWNLOAD_BASE` must
  use https (except on your own computer).
- **Claude Code plugin:** `/plugin marketplace add autobot-lab/fastpilot`, then
  `/plugin install fastpilot@fastpilot` adds the fastpilot tools and its 21 testing skills in one step.
- **GitHub Actions:** run your fastpilot web tests in CI with
  [`autobot-lab/fastpilot-action@v1`](https://github.com/autobot-lab/fastpilot-action).

### Clear messages
- **A missing secret stops the test before it starts.** When a test needs a secret such as
  `${DEMO_PASS}` that is not set, `fastpilot run` says "Could not run", names the secret (never its
  value), gives the exact command to set it, and does not retry. It is reported as a setup problem,
  not as a bug in your app, and never as a pass. Schedules and Release watch say "could not run:
  needs DEMO_PASS" in the summary and the notification, and the Schedules page shows it in amber with
  a link to Settings › Secret.
- The Ship check card keeps each test on one row.
- Live says where a session came from: "Started from the terminal" or "Scheduled · <schedule name>".
- Blocked actions are explained in plain words ("the built-in blocked list"), without setting names.
- A web-only run prints "Web · Chrome" instead of phone and iPhone counts.
- The demo pages work at phone width, so Ship check no longer flags every demo page.

### iPhone
- **The first iPhone test sets up iPhone testing by itself.** When the test runner is missing,
  `fastpilot run` (or **Run now**) builds it once with your Xcode, shows the time so far, and then
  starts the test. Without Xcode it says to install Xcode from the App Store.
- A scheduled run does not build it: it reports that a one-time setup is needed, so run one iPhone
  test by hand first.

### For maintainers
- The release workflow can publish to npm with Trusted Publishing (no stored token, with
  provenance) once the owner sets it up on npmjs.com and GitHub. See
  `docs/releasing.md`.

### Known limits
- iPhone sessions started from Claude Code or dashboard Chat do not build the runner yet: use **Set up
  iPhone testing** on the Device page, or `fastpilot setup ios`, once.
- `fastpilot run` reads secrets only from the environment, not from `.mcp.json`. The message says how
  to set them.
- Not tested yet: models other than Claude, Linux, physical Android phones and iPhones.

## What's new in 0.4.1

Use the phones you already have, and a quieter Chat.

### Devices
- **Your installed emulators come first.** When no Android device is running, the Device page lists the
  emulators already on this computer by name, each with **Start**. An emulator starts only when you
  click Start and confirm; Chat and agents cannot start one. It uses a free port, and other running
  emulators are not touched. **Add a new test phone** (a download of about 1.5 GB) is now the second
  choice, and the main one only when no emulator is installed.
- **Pick the iPhone simulator tests use.** The Device page lists your installed iPhone simulators.
  **Use for tests** saves one; **Any running simulator** goes back to the default.
- **iPhone testing is a one-time setup**, no longer shown as an error. `fastpilot doctor`, `fastpilot init`
  and the Device page say "iPhone testing needs a one-time setup (about 5 minutes)". The Device page has
  **Set up iPhone testing**, and `fastpilot init` offers it when Xcode is found.
- **`fastpilot run --workers` uses only the device you chose.** With `FASTPILOT_ANDROID_SERIAL` (or
  `ANDROID_SERIAL`) set, no other Android device is contacted. The first line of the run says which
  devices are used.

### Dashboard
- The empty Chat no longer shows the three suggestion buttons. It keeps the heading and one help line.

### Docs
- The README is now a step-by-step guide to the dashboard: start in 3 steps, then each page in turn.

### Fixes
- The release check reaches an Android device only in its Android step, and two tests that sometimes
  failed on a busy machine are stable.

### Known limits
- The iPhone test runner is not built automatically on the first iPhone test yet: run **Set up iPhone
  testing** (or `fastpilot setup ios`) once.
- The known limits of 0.4.0 below still apply.

## What's new in 0.4.0

Talk to it, in plain words. Several QA agents, each with its own model and skills, that test like a
certified tester and show what they covered.

### Agents
- **Several agents, like a team.** Each agent has its own Chat, Live view, Schedules, Instructions
  (with suggestions you can click), Skills, History and settings. Create one with a short form, or
  describe it in one sentence in Chat and click **Create**. Existing projects get one default agent
  that keeps your chat, schedules and history.
- **Provider, then model, per agent.** Claude Code, OpenAI, OpenRouter, Groq, Ollama (local) or another
  OpenAI-compatible service. API keys are stored once per provider in Settings. Dashboard Chat now
  works with any provider. Models other than Claude are labelled **"not tested yet"**.
- **Agents run in parallel.** Web tests run side by side. A phone or emulator runs one test at a time;
  other agents wait in a queue for it. "Tests at once" limits how many run together (default 3).

### Tests that think like a certified tester
- **21 built-in skills**, grouped by ISTQB® test design technique (fastpilot is not affiliated with
  ISTQB): Choose technique, Equivalence partitioning, Boundary value analysis, Decision table, State
  transition, Pairwise, Use case scenarios, Error guessing, Exploratory testing, Checklist-based (with
  checklists for login and OTP, forms, safe checkout, accessibility and mobile), Acceptance tests,
  Regression, Bug report, Test report and Release check. `fastpilot init` and `fastpilot demo` install
  them.
- **"Techniques used"**: results say which technique was used, which values were tried, what was
  assumed, and what was covered (for example "Boundary value analysis · Age — 6 of 6 values").
- **Regression** puts what is **newly failing since the last run** first, then still failing, flaky,
  blocked and passed.
- **Practice pages** in `fastpilot demo`: a ticket field with limits, shipping rules and an order
  status flow, each with one planted bug.

### Everything through chat
- **Chat creates a test** from a sentence. One card shows the steps; **Save & schedule** or **Save test
  only**. Nothing is written before you click, and variables the test needs are checked first.
- **All settings through chat**, in the dashboard and in Claude Code:
  - making limits stricter applies at once, with **Undo**;
  - other changes need your click;
  - **loosening a limit** only appears when you asked for it in your own words, as a red card that
    quotes you; allowing **Pay** also needs you to type "Pay" and expires after 10 minutes;
  - text on a tested page can never ask for a change;
  - in the terminal, the AI can only file a request; you approve it yourself with
    `fastpilot config approve`;
  - every change is written to an activity log.
- **"Remember this as a note"**: a card in Chat saves a project note from your own words.

### Schedules and releases
- **Every N hours** (1–24) with an active window (for example 08:00–20:00), **Run now**, and Next / Last
  columns.
- **The Schedules table says why a run failed** (which test, and the reason), not only "0 of 1 passed".
- **Release watch**: after a release, re-runs your tests every 5, 10, 15, 20, 30 or 60 minutes for 1 to
  24 hours. The first run is the "before". After that you hear once about each problem that is new
  since then (a test that starts failing, or the tests no longer running), on the schedule's
  notifications: Mac notification, Slack webhook or GitHub issue. When the time is up it turns itself
  off and removes its system-scheduler entry; the Schedules list keeps it with how it went. Start one
  from Schedules › New schedule › Release watch, from Chat, or with
  `fastpilot schedule watch tests/ --every 10 --for 2`.
- **Ship check**: `fastpilot ship-check <url>` (or a `shipCheck` step) looks for console errors,
  failed requests, broken links, layout at phone, tablet and desktop width, and basic accessibility,
  then gives one verdict: SHIP, SHIP WITH FIXES, DO NOT SHIP or INCONCLUSIVE, always with what was not
  checked. It exits with 0 (SHIP or SHIP WITH FIXES), 1 (DO NOT SHIP) or 2 (INCONCLUSIVE). Links are
  checked without your cookies unless you ask (`--cookies`); sign-out, delete, one-time and other
  links that change something are never opened.
- **In Chat**, a release check shows as a **verdict card**, labelled "the bot's summary" (the bot wrote
  it from the checks it ran; fastpilot does not verify it). A bug report shows as a **preview card**
  that is not sent anywhere, with Copy report and Copy as Markdown.

### Dashboard
- **History**: runs and saved Chat conversations; delete one, several or all; optional auto-delete
  after 7, 30 or 90 days.
- **Memory**: one line per test (Reliable, Unstable, Failing, Set aside, New), with **Set aside** and
  **Forget**, plus **project notes** (staging address, test account names, "login uses a code") that
  agents read. Notes that look like a password or hide invisible text are refused. Notes are kept in
  your own fastpilot folder, not in the project. An agent on another AI provider sends its notes to
  that provider.
- **Device: Fix buttons** for "Install Chrome" and "Set up an emulator" (you confirm the size and time;
  only a person can start them). Devices are named by type ("Android emulator", "iPhone simulator").
- After `fastpilot init` and `fastpilot demo`: "Open the dashboard now? [Y/n]".
- **Everything is in English**: dashboard, terminal messages and the guide.

### Setup
- **`fastpilot setup ios` works without the source code**: it builds the iPhone test runner once with
  your Xcode into `~/.fastpilot/ios-runner`.
- **Android tests fail clearly** when no emulator is there, instead of being skipped. They are also
  more stable on a busy machine.
- fastpilot checks your **Claude Code version** in `init`, `doctor`, Chat and `fastpilot run`. It needs
  2.1.280 or newer and tells you to run `claude update` when it is older.
- `fastpilot setup --help` and `fastpilot doctor --help` show help instead of running.

### Upgrading from 0.3.2
- Messages are now in English. New projects get `tests/example.md`; an existing `tests/contoh.md` is
  kept.
- Block words that start with `!`, the word `default`, and words with a comma are refused from chat
  and from agents. Only you can switch the built-in list off, in Settings.
- fastpilot writes `.fastpilot/.gitignore` in your project. It ignores everything except visual
  baselines, and never overwrites a file you already have.
- Project notes live in your own fastpilot folder (`~/.fastpilot/projects/`), not in the project, so a
  downloaded project cannot bring notes of its own.

### Fixes
- History serves and deletes only reports in fastpilot's own folders.
- A schedule saved while a run is finishing is no longer lost.
- Session ids are checked before they name a file.
- "Save note" works when you type into the empty form.

### Known limits
- `fastpilot run`, schedules and Release watch read secret variables such as `${DEMO_PASS}` only from
  the shell, not from `.mcp.json` (Chat and Claude Code do read `.mcp.json`). In the demo folder, run
  `export FASTPILOT_VARS=DEMO_PASS DEMO_PASS=<the demo password>` first, or a saved test fails with
  "variable DEMO_PASS is not allowed".
- Claude Code in the terminal reads project notes when it starts: restart it after you change a note.
  Dashboard Chat picks up changed notes on your next message.
- Ship check reads the page from inside the page, so a hostile page can fake its own layout and
  accessibility results (not which links are opened). Afterwards the screen size is put back from the
  page's own numbers, within safe bounds.

### Not in this release
- Jira and the Apps page (planned for 0.5).
- Not tested yet: models other than Claude, Linux, physical Android phones and iPhones.
