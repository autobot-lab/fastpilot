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
