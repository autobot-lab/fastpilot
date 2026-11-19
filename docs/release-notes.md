## What's new in 0.1.2

- `fastpilot doctor` checks web, Android, iOS and Claude Code, with a fix hint for each ✗;
  `fastpilot setup` applies the fixes it can, asking before each one.
- Settings file `~/.fastpilot/config.toml`, `fastpilot config list|get|set|unset|path`, and a
  `config` MCP tool. Environment variables still win. Guard rails (`allowed_domains`,
  `confirm_actions`, `vars`, `state_expire_days`) are pinned when fastpilot starts; an agent can only
  make them stricter.
- README rewritten for the npm / curl install.

Install (macOS arm64/x86_64, Linux x86_64):

```bash
curl -fsSL https://raw.githubusercontent.com/autobot-lab/fastpilot/main/scripts/install.sh | sh
npm install -g fastpilot
```

Every archive has a `.sha256` next to it; the install script and the npm package check it.
