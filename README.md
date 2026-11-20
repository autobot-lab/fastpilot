# fastpilot

**Test your web, Android, and iOS apps in plain language.**
You write what to check, and AI (Claude Code) runs it on your own computer.
Free, no cloud, and payment buttons are blocked from the start.

## What you need

- A Mac (or Linux)
- Google Chrome
- [Claude Code](https://claude.com/claude-code), signed in

## How to use it

**1. Install** (once)

```bash
npm install -g fastpilot
```

**2. Set up your project** (once per project)

```bash
cd ~/your-project
fastpilot init
```

`init` asks before each change; answer `y`. It connects fastpilot to Claude Code and creates
an example test, `tests/example.md`.

**3. Talk to fastpilot**

Open Claude in your project folder:

```bash
claude
```

Then just say what you want, in plain words. No commands, no code:

| You want to | Type |
|---|---|
| Check something quickly | *fastpilot, check that login on https://yourapp.com goes to the Dashboard* |
| Save a test for later | *fastpilot, create a test for the checkout flow and save it* |
| Run a regression | *fastpilot, run a regression* |
| Run part of it | *fastpilot, run the Android tests only* |
| Understand a failure | *Why did the checkout test fail? Show me the screenshot* |

fastpilot opens the app, does every step, and tells you what passed and what failed.
A regression runs every test you saved, so save a few tests first.

## Open the dashboard

The dashboard is a page in your browser where you can chat, watch tests run, schedule them, and
change settings, without typing commands in Claude.

1. Open the **Terminal** app.
2. Type this and press Enter:

   ```bash
   fastpilot ui
   ```

3. Your browser opens the dashboard by itself.
   If it does not, the Terminal shows a link like `http://127.0.0.1:53712/?code=...`.
   Copy it into your browser.
4. Keep the Terminal open while you use the dashboard. To close it, press `Ctrl-C` in the Terminal.

Good to know:

- The link works **once** and for **10 minutes**. If it expired, or you closed the page, press
  `Ctrl-C` and run `fastpilot ui` again for a new link.
- The dashboard only works on this computer, not from your phone or another computer.
- Open it yourself. Do not ask the AI to open it for you: the dashboard can loosen the limits.

## What's in the dashboard

| Menu | What it's for |
|---|---|
| **Chat** | Ask for a test in plain words, e.g. *"Test https://example.com and check that 'Example Domain' is shown"* |
| **Live** | Watch tests run, and take over if needed |
| **Jadwal** (Schedules) | Run tests automatically every day or every week. If one fails, you get a Slack message or a Mac notification |
| **Device** | Check whether Chrome, Android, and iOS are ready |
| **Pengaturan** (Settings) | Pick the AI model, set limits, store secrets |

## Write a test yourself (optional)

You don't have to: fastpilot can write tests for you. If you want to, one test is one `.md` file
in the `tests/` folder:

```markdown
# Search for a product
id: search-product
platform: web
target: https://yourshop.com
- Type "shoes" in the search box and press Enter
- Check that a list of search results is shown
```

No selectors or code. Write it like you would explain it to a friend.
`platform` can be `web`, `android`, or `ios`.

## Safe from the start

- Buttons like **pay, buy, checkout, transfer, delete account** are always blocked.
- Plain **delete/remove** buttons must be confirmed first.
- Passwords are written as `${NAME}` and always show as `***` in results ([how to store them](docs/guide.md#menulis-test-sendiri)).
- The AI can only make the rules stricter. To loosen one, **you** approve it: on a red card in Chat (it quotes what you asked, and Pay needs the word typed), or with `fastpilot config approve` in your own terminal. Every change is kept in `~/.fastpilot/audit.jsonl`.

Want to test a checkout? Ask in Chat: *"Allow the Pay button for this project"*, or open `fastpilot ui` → **Settings › Limits & permissions** and remove the word from the list.

These limits protect you from what a tested page tells the AI. They are not a lock against programs on your own computer: anything that runs as you can edit `~/.fastpilot/config.toml`. If you let Claude Code run `fastpilot config` commands (`Bash(fastpilot config:*)`), that also covers `fastpilot config approve`; keep it out of your allowed commands.

## If you get stuck

Run this first:

```bash
fastpilot doctor
```

`doctor` shows what isn't ready yet and how to fix it.

| Problem | Try |
|---|---|
| Chrome not found | `fastpilot setup web` |
| fastpilot doesn't show up in Claude | Run `fastpilot init` again |
| The dashboard link doesn't work | It works once, for 10 minutes. Press `Ctrl-C`, then run `fastpilot ui` again |
| Android isn't ready | `fastpilot setup android`, then `fastpilot doctor android` |
| A test stops with "Diblokir Batasan" (blocked by limits) | That's on purpose. Change it in the dashboard › Pengaturan › Batasan & izin |

## Learn more

More details, including the technical ones, are in the **[full guide](docs/guide.md)** (currently in Indonesian):

- [Android](docs/guide.md#android) and [iOS](docs/guide.md#ios-menyusul-di-v033-sekarang-dari-checkout)
- [Claude Desktop](docs/guide.md#claude-desktop)
- [All settings](docs/guide.md#pengaturan)
- [Devices on another machine](docs/guide.md#device-di-mesin-lain) and [physical phones](docs/real-devices.md)
- [Known limits](docs/guide.md#batasan-yang-diketahui)

License: [Apache-2.0](LICENSE)
