# fastpilot

AI-driven QA for web, Android and iOS: an MCP server for Claude Code and other MCP clients,
plus a shell CLI. This package downloads the prebuilt binary for your platform (macOS arm64/x64,
Linux x64) from the project's GitHub releases and checks its SHA-256.

```bash
npm install -g fastpilot
claude mcp add fastpilot -- fastpilot mcp
fastpilot open https://example.com && fastpilot snapshot -i
```

Documentation: https://github.com/autobot-lab/fastpilot
