# Security policy

## Supported versions

Only the latest release on npm (`fastpilot@latest`) and GitHub Releases gets security fixes.

## Reporting a vulnerability

Please do **not** open a public issue, pull request or discussion for a security problem.

Report it privately: **Security › Report a vulnerability** on
https://github.com/autobot-lab/fastpilot/security/advisories/new

Include what you found, the steps to reproduce it, the fastpilot version (`fastpilot --version`)
and your OS. Never include real passwords, API keys or tokens: use fake ones.

What to expect:
- We confirm we got your report within 3 working days.
- We tell you whether we accept it and our plan within 10 working days.
- We fix accepted issues in a patch release and credit you in the advisory, unless you ask us not to.

## Scope

fastpilot runs on the user's own machine. In scope: the local dashboard (CSRF, DNS rebinding,
origin checks), prompt injection from page, ticket or app text that leads to an action the user
did not ask for, secret storage and redaction, commands built from input (shell, adb, simctl),
file paths and archives, API keys sent to the wrong address, and the release chain (GitHub
Actions, npm, embedded binaries).

Out of scope: attacks that need an already compromised machine or user account, and bugs in a
model provider's own service.
