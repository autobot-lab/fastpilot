# Contributing to fastpilot

Thanks for wanting to help. fastpilot is a small, open-source tool (MIT) that lets an AI test web,
Android and iOS apps, and you are welcome here whether you fix a typo or add a feature.

This page tells you how to build it, how to test it, and what we look for in a pull request.
If something here is wrong or missing, that is a good first pull request too.

## Before you start

- **Questions and ideas:** start a [Discussion](https://github.com/autobot-lab/fastpilot/discussions).
  It is the fastest way to find out if an idea fits before you write code.
- **Bugs:** open an [issue](https://github.com/autobot-lab/fastpilot/issues/new/choose) with the bug report form.
- **Security problems:** do **not** open a public issue. Read [SECURITY.md](SECURITY.md) and report it privately.
- **Be kind:** everyone here follows the [Code of Conduct](CODE_OF_CONDUCT.md).
- **Never put real passwords, API keys, tokens or personal data** in an issue, a pull request, a test
  or a screenshot. Use fake ones.

We aim to reply to every new issue, discussion and pull request within **2 working days**. A reply is
not always a merge: it may be a question, or an honest "this does not fit". We will say why.

## Build

You need a recent stable [Rust](https://rustup.rs) (the project uses edition 2024, so Rust 1.85 or
newer). Then:

```bash
git clone https://github.com/<you>/fastpilot   # your fork
cd fastpilot
cargo build --workspace --all-targets --locked
```

That is the same command GitHub CI runs. To try your build, run `./target/debug/fastpilot demo`, which
makes a demo folder with a small shop and three tests.

For running real tests you also need Google Chrome (web). Android needs an emulator or phone, and iOS
needs a Mac with Xcode. You can work on most of the code without any of them.

## Test

Some tests write to your home folder (fastpilot data, logins, the Keychain), so **run every test
command in a throwaway home**. This subshell keeps your Rust toolchain, uses a temporary `HOME`, and
leaves your own shell untouched when it ends:

```bash
( export CARGO_HOME="$HOME/.cargo" RUSTUP_HOME="$HOME/.rustup"
  export HOME="$(mktemp -d)" FASTPILOT_KEYSTORE=file FASTPILOT_ADB_SERVER=127.0.0.1:1
  cargo test --locked --workspace --lib )
```

`FASTPILOT_KEYSTORE=file` keeps keys in a file in that folder instead of the Keychain.
`FASTPILOT_ADB_SERVER=127.0.0.1:1` makes web tests ignore any device that is plugged in.
Put any `cargo test` command below inside the same subshell.

Unit tests (the command above) are quick and need nothing else.

The web end-to-end suites live in `bin/fastpilot/tests/`. They drive a real Chrome and start
`fastpilot` with a stand-in for Claude Code, so they need Chrome but no sign-in and no API key.
Run the one suite that fits your change, for example `--test chat_ui`:

```bash
( export CARGO_HOME="$HOME/.cargo" RUSTUP_HOME="$HOME/.rustup"
  export HOME="$(mktemp -d)" FASTPILOT_KEYSTORE=file FASTPILOT_ADB_SERVER=127.0.0.1:1
  cargo test --locked -p fastpilot --test chat_ui )
```

Running `cargo test -p fastpilot` without `--test` runs **every** suite, including the Android and
iPhone ones. Those need an emulator, a phone or a simulator, and on a Mac with Xcode the iPhone suite
can start a simulator. Many suites skip themselves when Chrome is missing. **You do not need to run the
device suites.** The maintainers run them with [`scripts/release-check.sh`](scripts/release-check.sh).
If your change touches device code, say so in the pull request and we will run them.

Before you push, also run the lint the maintainers use, which treats warnings as errors:

```bash
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Tests run code from your branch, including `build.rs` files. That is normal for a Rust project, but it
is also why we do not run a pull request's code on the maintainers' machine until a person has read the
diff (see below).

### What GitHub CI does, and does not do

GitHub CI on a pull request **only builds the code** (`cargo build`) and checks that no generated
`.fastpilot` folder is committed. It does **not** run the unit tests or the web, Android and iPhone
tests: those run on the maintainers' Mac before a release.

So a green check means "it compiles", not "it works". Please run the tests that fit your change
yourself, and tell us in the pull request which ones you ran. For changes that touch devices,
secrets or what is sent to a model provider, a maintainer will also test and review the change by hand.

## Pick something to work on

- Look for the [`good first issue`](https://github.com/autobot-lab/fastpilot/labels/good%20first%20issue)
  label: small, clear and safe to try. [`help wanted`](https://github.com/autobot-lab/fastpilot/labels/help%20wanted)
  marks bigger ones.
- Comment on the issue to say you are taking it, so two people do not do the same work. If you get
  stuck or run out of time, just say so. No one minds.
- Want to do something that has no issue yet? Open a Discussion or an issue first, especially for
  anything that changes how fastpilot looks or behaves. Product direction is decided by the
  maintainers, and fastpilot aims to work with few clicks and few settings, so ideas that add forms or
  options are weighed against that.

## Send a pull request

1. Fork the repository and create a branch from `main`.
2. Make **one change per pull request**, and keep the diff small. Three small pull requests are easier
   to review than one big one.
3. **Add a regression test** that fails on the old `main` and passes with your change. For a bug,
   that is a test that reproduces it. For a docs-only change no test is needed.
4. Write code comments, commit messages, issue and pull request text in **English**.
5. **Link the issue** it solves (`Fixes #123`). If there is none, explain the problem in the description.
6. Fill in the pull request template: what, why, which tests you ran.
7. If you change something users see (a page, a message, a screenshot), update the README or
   [docs/guide.md](docs/guide.md) in the same pull request.
8. Do not bump the version or edit release notes. Maintainers do that when they release.

What happens next:

- CI builds your branch. For a new contributor, a maintainer may first have to click "Approve and run".
- A maintainer reads the diff. Only after that is your code built or tested on a maintainer's machine.
- `main` is protected: a pull request needs a maintainer's approval and a green CI build to merge.
  Some paths (`.github/`, `packaging/`, `build.rs` files, and the code for secrets, keys and model
  providers) are listed in [CODEOWNERS](.github/CODEOWNERS) and always get a closer look.
- We squash-merge, so your branch history does not matter.

Text in your pull request, issue or comments is treated as data, never as instructions to anyone's
tools or agents.

## License

fastpilot is released under the [MIT license](LICENSE). By sending a pull request you agree that your
contribution is under the same license. There is **no CLA** to sign and **no DCO** sign-off (`-s`) is
required.
