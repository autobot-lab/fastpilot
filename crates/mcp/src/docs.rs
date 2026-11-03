//! On-demand reference served by the `docs` tool, so the client system prompt stays small.

pub fn topic(name: &str) -> Option<&'static str> {
    Some(match name {
        "dsl" => DSL,
        "selectors" => SELECTORS,
        "parallel" => PARALLEL,
        "examples" => EXAMPLES,
        "errors" => ERRORS,
        "memory" => MEMORY,
        _ => return None,
    })
}

const DSL: &str = r#"# Flow DSL (Maestro-compatible subset + fastpilot extensions)

A flow is a YAML list. Each step has exactly one command. Quote numeric text ("+62812", "1.10").

| Step | Form | Platforms |
|---|---|---|
| launchApp | `launchApp: com.example.app` or `pkg/.Activity` (stop + start) | android, ios |
| openUrl | `openUrl: https://...` or a local path (`examples/demo/login.html`) | web (android/ios: deep link) |
| tapOn | `tapOn: "Login"` / `{ ref: e3 }` / `{ id: btn }` / `{ role: button, name: "Login" }` / `{ point: [x, y] }` | all |
| inputText | `inputText: { into: "Email", text: "a@b.com" }` (replaces content); `inputText: "abc"` types into the focused field | all |
| assertVisible / assertNotVisible | `assertVisible: "Dashboard"` or `{ text: "Dashboard", timeout: 10000 }` (ms) | all |
| scroll | `scroll: down` or `{ direction: up, within: "Daftar" }` | all |
| swipe | `swipe: { from: [x1, y1], to: [x2, y2] }` | android, ios |
| back | `back` | all |
| pressKey | `pressKey: Enter` (Tab, Escape, Backspace, Delete, Space, ArrowUp/Down/Left/Right, Home, End, PageUp, PageDown) | all |
| selectOption | `selectOption: { from: "Negara", option: "Indonesia" }` (native `<select>`) | web |
| uploadFile | `uploadFile: { into: "Lampiran", files: ["a.pdf"] }` | web |
| evalScript | `evalScript: "document.title"` (value is returned) | web, android WebView |
| session | `session: web1` (switch) or `session: { start: web1, platform: web, target: "...", reset: true }` | — |
| parallel | see docs(topic: parallel) | — |

Rules
- Auto-wait before/after every step (actionability, DOM/accessibility quiet, navigation and requests
  triggered by the action). There is no sleep.
- Element lookup retries until 5 s (assertVisible: 10 s); actions are never re-sent.
- The first failing step stops the run and returns reason, category, candidates and a snapshot.
- JS dialogs (alert/confirm/beforeunload) are accepted automatically and show up as `alert "confirm: ..."`.
- `${NAME}` is replaced from fastpilot's environment, only for names listed in `FASTPILOT_VARS`;
  values are masked (`***`) in everything returned.
- Android WebView content is merged into the snapshot and driven over CDP (tapOn, inputText, evalScript).
"#;

const SELECTORS: &str = r#"# Selectors

Resolution order: ref (`e12`, from the latest snapshot) → visible text / label → role + name → id → point.
Never XPath.

- Strings shaped like `e12` are refs; other strings are text.
- `id` matches DOM id (web), resource-id without package (android), accessibilityIdentifier (ios).
- Text matches exact, then case-insensitive exact, then case-insensitive "contains".
- tapOn prefers interactive nodes, inputText prefers text fields.
- Several exact matches: the first (topmost) wins. Several fuzzy matches: the step fails `ambiguous`
  and lists candidates; pick one by ref.
- Refs follow the element while it exists (session-scoped); a ref never jumps to another element.
- `snapshot { filter: "batal" }` finds nodes with their ancestors; `coords: true` adds boxes.

Apps without accessibility labels (`low_label_coverage` in the snapshot):
- Jetpack Compose: set `Modifier.semantics { testTagsAsResourceId = true }` on the root and
  `Modifier.testTag("login")`, then use `{ id: login }`.
- Flutter: wrap widgets in `Semantics(label: ...)` / set `identifier`; Flutter builds semantics only
  while an accessibility client is connected (the agent's UiAutomation counts as one).
- Otherwise: `screenshot { annotate: som }` lists every target box; tap with `{ point: [x, y] }`.
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

const MEMORY: &str = r#"# Test runs and path memory

1. `session start` with `test_id` → the response carries a hint: the last passing flow for that test.
2. Send the hint as one `run_flow` with the same `test_id`. If a step fails, explore from the returned
   snapshot and continue with more `run_flow`/`assert` calls (all with `test_id`).
3. `report { test_id, status: pass|fail, notes }` closes the run: `.fastpilot/runs/<utc>-<test_id>/`
   gets `report.md`, `steps.jsonl`, `assertions.json`, and `fail-N.jpg|txt` evidence.
4. Only a run reported `pass` with no failed step/assert becomes the new hint. Refs are saved as
   role+name selectors. A failed run only adds to the statistics.

`FASTPILOT_MEMORY=readonly` (CI: replay reviewed hints, never rewrite) or `off`. `FASTPILOT_DIR` moves `.fastpilot/`.
"#;

const ERRORS: &str = r#"# Failure reasons

Every failure also has a category: `app` (screen not as expected: product bug or changed UI),
`infra` (browser/device/transport), `flow` (the flow is wrong or uses an unsupported step).

| Reason | Meaning | What to do |
|---|---|---|
| not_found | no node matched before the timeout | use a candidate ref, or assertVisible with a longer timeout |
| ambiguous | several fuzzy matches | pick one of the listed refs |
| not_actionable | hidden, disabled, animating or covered | dismiss the overlay / wait for the state with assertVisible |
| timeout | the driver did not answer in time | retry; check the page |
| unsupported | step or platform not available on this driver/phase | use a supported step |
| script_error | evalScript threw | fix the expression |
| session | unknown / duplicate session | start or switch sessions |
| transient | page/screen was mid-transition (retried until timeout) | usually resolves; check navigation |
| assertion_failed | assert tool: property did not hold | compare text in the returned snapshot |
| invalid_flow | bad YAML or step | run check_flow |
| driver_error | CDP/browser failure (e.g. Chrome exited, navigation error) | restart the session |
"#;
