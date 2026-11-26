## What

<!-- What does this change, in one or two sentences? -->

## Why

<!-- The problem it solves. Link the issue: Fixes #123 -->

## Test

<!-- The regression test you added (it should fail on the old main), and which tests you ran:
     cargo test --locked --workspace --lib, a web suite, clippy ... Docs-only? Say so. -->

## Checklist

- [ ] One change in this pull request, with a small diff
- [ ] A regression test that fails on the old `main` (or this is docs only)
- [ ] I ran the tests that fit my change (GitHub CI only builds the code)
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` passes
- [ ] Text, comments and commit messages are in English
- [ ] No real passwords, keys, tokens or personal data anywhere (code, tests, screenshots)
- [ ] README or `docs/guide.md` updated if users see a difference
- [ ] I did not change the version or the release notes
