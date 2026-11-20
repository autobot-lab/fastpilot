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
an example test, `tests/contoh.md`.

**3. Write your test**

Open `tests/contoh.md` and change `target` to your app's address (see [Write a test](#write-a-test)).

**4. Run it**

```bash
claude
```

In Claude, type: *Use fastpilot, run tests/contoh.md and then report*

Claude runs the test in Chrome and tells you whether it passed. The report is saved in
`.fastpilot/runs/`.

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

## Write a test

One test = one `.md` file in the `tests/` folder:

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
- The AI can only make the rules stricter, never looser.

Want to test a checkout? Open `fastpilot ui` → **Pengaturan › Batasan & izin** (Settings › Limits) and remove the word from the list.

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

Everything else is in the **[full guide](docs/guide.md)** (currently in Indonesian):

- [Android](docs/guide.md#android) and [iOS](docs/guide.md#ios-menyusul-di-v033-sekarang-dari-checkout)
- [Claude Desktop](docs/guide.md#claude-desktop)
- [All settings](docs/guide.md#pengaturan)
- [Run tests without opening Claude (`fastpilot run`)](docs/guide.md#tanpa-membuka-claude-code-fastpilot-run)
- [One command per step in the terminal (shell CLI)](docs/guide.md#shell-cli-satu-perintah-per-langkah)
- [Devices on another machine](docs/guide.md#device-di-mesin-lain) and [physical phones](docs/real-devices.md)
- [Build from source](docs/guide.md#build-dari-source)
- [Known limits](docs/guide.md#batasan-yang-diketahui)

License: [Apache-2.0](LICENSE)
