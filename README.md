# fastpilot

**Test your web, Android, and iOS apps in plain language, from one dashboard.**
You say what to check; AI runs it on your own computer, shows you every step live, and tells you
what passed and what failed. Free, open source, no cloud. Payment and delete-account buttons are
blocked from the start.

<img src="docs/images/07-chat-save-test.png" width="860" alt="The fastpilot dashboard: an agent called QA Checkout gets one sentence in Chat and answers with a Save test and schedule card that lists the test steps, a schedule every 2 hours, and the buttons Save and schedule, Save test only and Edit.">

## Start in 3 steps

You need a Mac (or Linux), Google Chrome, and [Claude Code](https://claude.com/claude-code) signed in.

**1. Install** (once)

```bash
npm install -g fastpilot
```

**2. Set up your project** (once per project)

```bash
cd ~/your-project
fastpilot init
```

`init` asks before each change; answer `y`. At the end it asks **"Open the dashboard now?"**:
press Enter.

> Using Claude Code? You can also install fastpilot as a plugin (the MCP server and its 21 testing
> skills in one step): `/plugin marketplace add autobot-lab/fastpilot`, then `/plugin install fastpilot@fastpilot`.

> Just want to look around first? Run `fastpilot demo` instead. It makes a demo folder with demo
> pages and example tests, and opens the dashboard on it.

**3. Open the dashboard** (every time)

```bash
fastpilot ui
```

Your browser opens the dashboard. Keep the Terminal open while you use it; `Ctrl-C` closes it.

- The link works **once**, for **10 minutes**. If it expired, press `Ctrl-C` and run `fastpilot ui` again.
- The dashboard only works on this computer.
- Open it yourself; do not ask the AI to open it for you.

Everything else happens in the dashboard.

## Guide: your first test in the dashboard

### 1. Check your devices: **Device**
Shows whether Chrome, Android, and iOS are ready, with a ✓ or the reason why not.
- **Install Chrome** and **Set up an emulator** are one-click **Fix** buttons. Each asks you first,
  showing the size and the time it takes.
- **iPhone** testing needs a one-time setup with your Xcode (about 5 minutes).
- Only web? Chrome is enough.

<img src="docs/images/02-device.png" width="860" alt="fastpilot's Device page: two Android emulators installed on this computer, each with a Start button, and the iPhone simulators tests can use.">

### 2. Pick the AI and store passwords: **Settings**
- **AI model:** pick the provider, then the model. Claude Code is the default. Other providers
  (OpenAI, OpenRouter, Groq, Ollama) also work, and are marked "not tested yet".
- **Secret:** add the names of passwords your tests need, for example `TEST_PASS`. Tests write them as
  `${TEST_PASS}`, and they always show as `***`. Never type a password in Chat.
- **Limits & permissions:** see the next section, "Safe from the start".

### 3. Make an agent: **Agents › New agent**
An agent is a tester with its own job, devices, instructions, and skills.
- Fill in three things: what it tests, where (Web, Android emulator, iPhone simulator…), and its
  instructions. Click a suggested instruction to add it.
- **Skills** are ready-made test know-how, grouped by technique: boundary values, decision tables,
  regression, safe checkout, and more. Suggested skills are already ticked, each with a reason.
- Or simply describe the agent in one sentence in Chat and click **Create**.

You can have several agents, for example "Checkout" on web and "Login" on Android. Each has its own
**Chat, Live, Schedules, Instructions, Skills, History** and **Agent settings**.

### 4. Ask for a test: **Chat**
Write what you want in plain words:

| You want to | Type in Chat |
|---|---|
| Check something now | *Check that login on https://yourapp.com goes to the Dashboard* |
| Save a test | *Create a test for the checkout flow and save it* |
| Test a field properly | *Test the quantity field properly* (it picks the technique and shows what it covered) |
| Run everything | *Run a regression* (newly failing tests come first) |
| Before a release | *Is staging ready to ship?* (one verdict: SHIP, SHIP WITH FIXES, DO NOT SHIP) |
| Understand a failure | *Why did the checkout test fail?* |

When a test is ready, a card shows its steps: **Save & schedule** or **Save test only**. Nothing is
saved before you click. The picture at the top of this page shows such a card.

### 5. Watch it run: **Live**
Watch every test step by step, on web, Android, and iOS. **Take over** if you need to tap something
yourself, then hand it back to the AI. Several agents can run at the same time; a phone runs one
test at a time and the others wait their turn.

<img src="docs/images/08-live.png" width="860" alt="The Live page: the QA Checkout agent is testing the event tickets page in Chrome; four steps are done and the fifth, waiting for the reservation message, is running, with Enlarge and Take over buttons.">

### 6. Run it automatically: **Schedules**
- **Every N hours** (with active hours, for example 08:00–20:00), every day, or every week.
- **Run now** for any schedule.
- **Release watch:** after a release, re-check every few minutes for a few hours and only tell you
  about new problems.
- Get notified by Slack webhook, Mac notification, or GitHub issue when something fails.
- Or just ask in Chat: *"run the checkout tests every 2 hours"*.

<img src="docs/images/09-schedules.png" width="860" alt="The Schedules page: a test that runs every 2 hours from 08:00 to 20:00 and a release watch that checks every 10 minutes, each with its next run, a passed last run and a Mac notification on failure.">

### 7. See results: **History** and **Memory**
- **History:** every run and every Chat conversation, with report, screenshots, and video. Delete
  one, several, or all, or let them be deleted after 7, 30, or 90 days.
- **Memory:** how each test is doing (Reliable, Unstable, Failing, Set aside), plus **project notes**
  every agent reads, for example *"Staging is https://staging.yourapp.com"* or *"Login uses a code"*.

## Dashboard map

| Menu | What it's for |
|---|---|
| **Agents** | Your testers: Chat, Live, Schedules, Instructions, Skills, History, Agent settings. Also the skill **Library** |
| **Live** | Every running test, from all agents |
| **Schedules** | All schedules and release watches |
| **Memory** | Test health and project notes |
| **History** | Past runs and conversations |
| **Device** | Chrome, Android, iOS readiness and Fix buttons |
| **Settings** | AI model, Limits & permissions, Secret |

## Safe from the start

- Buttons like **pay, buy, checkout, transfer, delete account** are always blocked.
- Plain **delete/remove** buttons must be confirmed first.
- The AI can make the rules stricter, never looser. To loosen one, **you** approve it: Chat shows a red
  card that quotes what you asked, and allowing **Pay** also needs you to type "Pay". Text on a tested
  page can never ask for a change.
- Every change is kept in `~/.fastpilot/audit.jsonl`.

Want to test a checkout for real? Ask in Chat: *"Allow the Pay button for this project"*, or open
**Settings › Limits & permissions**.

These limits protect you from what a tested page tells the AI. They are not a lock against programs
on your own computer: anything that runs as you can edit `~/.fastpilot/config.toml`.

## Prefer the terminal?

Everything also works from Claude Code. Open Claude in your project folder (`claude`) and say it in
plain words, for example *"fastpilot, run a regression"* or *"fastpilot, test the login page"*.
Results, schedules, and history show up in the dashboard too.

In GitHub Actions, use [`autobot-lab/fastpilot-action@v1`](https://github.com/autobot-lab/fastpilot-action):
it runs your tests on every pull request and adds the JUnit and HTML reports.

## If you get stuck

Open **Device** in the dashboard, or run:

```bash
fastpilot doctor
```

| Problem | Try |
|---|---|
| The dashboard link doesn't work | It works once, for 10 minutes. Press `Ctrl-C`, then run `fastpilot ui` again |
| Chrome or Android isn't ready | **Device** › Fix buttons, or `fastpilot setup web` / `fastpilot setup android` |
| iPhone isn't ready | `fastpilot setup ios` (once, with Xcode) |
| fastpilot doesn't show up in Claude | Run `fastpilot init` again |
| A test stops with "Blocked by limits" | That's on purpose. See **Settings › Limits & permissions** |
| A saved test can't find its password in `fastpilot run` | Export it first, e.g. `export FASTPILOT_VARS=TEST_PASS TEST_PASS=...` |

## Learn more

- **[Full guide](docs/guide.md)**: writing tests by hand, all settings, Android, iOS, Claude Desktop,
  devices on another computer, known limits
- [Physical phones](docs/real-devices.md)
- [Release notes](https://github.com/autobot-lab/fastpilot/releases)

License: [Apache-2.0](LICENSE)
