# fastpilot

**Test your web, Android, and iOS apps in plain language.**
You write what to check, and AI (Claude Code) runs it on your own computer.
Free, no cloud, and payment buttons are blocked from the start.

## What you need

- A Mac (or Linux)
- Google Chrome
- [Claude Code](https://claude.com/claude-code), signed in

## Get started in 3 steps

**1. Install**

```bash
npm install -g fastpilot
```

**2. Try the demo**

```bash
fastpilot demo
cd fastpilot-demo
claude
```

In Claude, type: *Use fastpilot, run tests/login.md and then report*

When you see ✓ `login-ok` passed, fastpilot works. 🎉

**3. Use it in your project**

```bash
cd ~/your-project
fastpilot init
```

Open `tests/contoh.md`, change `target` to your app's address, then ask Claude to run it.

## Use the dashboard (easiest)

```bash
fastpilot ui
```

The dashboard opens in your browser. Keep the Terminal open while you use it.

| Menu | What it's for |
|---|---|
| **Chat** | Ask for a test in plain words, e.g. *"Test https://example.com and check that 'Example Domain' is shown"* |
| **Live** | Watch tests run, and take over if needed |
| **Jadwal** (Schedules) | Run tests automatically every day or every week. If one fails, you get a Slack message or a Mac notification |
| **Device** | Check whether Chrome, Android, and iOS are ready |
| **Pengaturan** (Settings) | Pick the AI model, set limits, store secrets |

The dashboard link works once and for 10 minutes. If it expires, run `fastpilot ui` again.

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
| Android isn't ready | `fastpilot setup android`, then `fastpilot demo --android` |
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
