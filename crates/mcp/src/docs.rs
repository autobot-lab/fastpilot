//! On-demand reference served by the `docs` tool, so the client system prompt stays small.

pub fn topic(name: &str) -> Option<&'static str> {
    Some(match name {
        "dsl" => DSL,
        "selectors" => SELECTORS,
        "parallel" => PARALLEL,
        "examples" => EXAMPLES,
        "errors" => ERRORS,
        _ => return None,
    })
}

const DSL: &str = r#"# Flow DSL (Maestro-compatible subset + fastpilot extensions)

A flow is a YAML list. Each step has exactly one command.

| Step | Form |
|---|---|
| openUrl | `openUrl: https://...` or a local path (`examples/demo/login.html`) |
| tapOn | `tapOn: "Login"` / `{ ref: e3 }` / `{ id: btn }` / `{ role: button, name: "Login" }` / `{ point: [x, y] }` |
| inputText | `inputText: { into: "Email", text: "a@b.com" }` (replaces content); `inputText: "abc"` types into the focused field |
| assertVisible | `assertVisible: "Dashboard"` or `{ text: "Dashboard", timeout: 10000 }` (ms) |
| scroll | `scroll: down` or `{ direction: up, within: "Daftar" }` |
| back | `back` |
| evalScript | `evalScript: "document.title"` (value is returned) |
| session | `session: web1` (switch) or `session: { start: web1, platform: web, target: "https://..." }` |
| parallel | see docs(topic: parallel) |

Rules
- Auto-wait before/after every step (actionability, DOM quiet, requests triggered by the action). There is no sleep.
- Element lookup retries until 5 s (assertVisible: 10 s); actions are never re-sent.
- The first failing step stops the run and returns reason, candidates and a snapshot.
- `${NAME}` is replaced from fastpilot's environment and never echoed back.
- Not yet: launchApp, assertNotVisible, swipe and other Maestro commands (check_flow says so).
"#;

const SELECTORS: &str = r#"# Selectors

Resolution order: ref (`e12`, from the latest snapshot) → visible text / label → role + name → id → point.
Never XPath.

- Strings shaped like `e12` are refs; other strings are text.
- Text matches exact, then case-insensitive exact, then case-insensitive "contains".
- tapOn prefers interactive nodes, inputText prefers text fields.
- Several exact matches: the first (topmost) wins. Several fuzzy matches: the step fails `ambiguous`
  and lists candidates; pick one by ref.
- Refs are stable hashes of role + name + path, valid for the session while the node exists.
"#;

const PARALLEL: &str = r#"# Multi-session and parallel

```yaml
- session: { start: admin, platform: web, target: "https://admin.example.test" }
- session: { start: buyer, platform: web, target: "https://shop.example.test" }
- parallel:
    - session: admin
      steps:
        - tapOn: "Pesanan"
    - session: buyer
      steps:
        - tapOn: "Keranjang"
- session: admin
- assertVisible: "Pesanan"
```

- Each web session is its own BrowserContext (isolated cookies/storage) in one Chrome process.
- A branch is bound to one session; no `session` steps or nested `parallel` inside.
- Fail-fast: the first failing branch cancels the others; every touched session still reports state.
- Results are prefixed with the session id: `[admin] screen "..."`.
"#;

const EXAMPLES: &str = r#"# Examples

Login in one round-trip:

```yaml
- session: { start: web1, platform: web, target: "examples/demo/login.html" }
- inputText: { into: "Email", text: "a@b.com" }
- inputText: { into: "Password", text: "${DEMO_PASS}" }
- tapOn: "Masuk"
- assertVisible: "Budi"
```

Response:

```
ok 5/5 steps 1.70s
  ✓ 1 [web1] session start web1 (web) 1063ms
  ✓ 2 [web1] inputText "Email" "a@b.com" 78ms
  ✓ 3 [web1] inputText "Password" "${DEMO_PASS}" 84ms
  ✓ 4 [web1] tapOn "Masuk" 83ms
  ✓ 5 [web1] assertVisible "Budi" 385ms
[web1] screen "Dashboard" (file:///.../login.html)
e7 heading "Dashboard"
e8 text "Selamat datang,"
e9 text "Budi"
e10 button "Keluar"
```
"#;

const ERRORS: &str = r#"# Failure reasons

| Reason | Meaning | What to do |
|---|---|---|
| not_found | no node matched before the timeout | use a candidate ref, or assertVisible with a longer timeout |
| ambiguous | several fuzzy matches | pick one of the listed refs |
| not_actionable | hidden, disabled, animating or covered | dismiss the overlay / wait for the state with assertVisible |
| timeout | the driver did not answer in time | retry; check the page |
| unsupported | step or platform not available on this driver/phase | use a supported step |
| script_error | evalScript threw | fix the expression |
| session | unknown / duplicate session | start or switch sessions |
| invalid_flow | bad YAML or step | run check_flow |
| driver_error | CDP/browser failure (e.g. Chrome exited, navigation error) | restart the session |
"#;
