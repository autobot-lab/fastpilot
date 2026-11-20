//! S4: the technique practice pages of `fastpilot demo` behave as their rules say, except for the
//! one planted bug each (docs/demo-planted-bugs.md). Driven through fastpilot's own MCP tools in
//! Chrome, the way a skill would test them; a change that fixes or moves a bug by accident fails
//! here. Skips without Chrome.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::json;
use std::path::Path;

type Client = RunningService<RoleClient, ()>;
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn chrome_available() -> bool {
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"]
        .iter()
        .any(|p| Path::new(p).exists())
}

async fn flow(client: &Client, steps: &str) -> (bool, String) {
    let res: CallToolResult = client
        .call_tool(CallToolRequestParams::new("run_flow").with_arguments(json!({ "flow": steps, "session": "t" }).as_object().cloned().unwrap_or_default()))
        .await
        .expect("tool call");
    let text = res.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n");
    (res.is_error != Some(true), text)
}

async fn ok(client: &Client, steps: &str) {
    let (ok, out) = flow(client, steps).await;
    assert!(ok, "{steps}\n{out}");
}

async fn eval(client: &Client, js: &str) -> String {
    let (ok, out) = flow(client, &format!("- evalScript: {}\n", serde_json::to_string(js).expect("js"))).await;
    assert!(ok, "{out}");
    out.lines().find_map(|l| l.rsplit_once(" = ").map(|(_, v)| v.trim().trim_matches('"').to_owned())).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn practice_pages_keep_their_rules_and_their_one_bug() {
    if !chrome_available() {
        return eprintln!("skipping: no Chrome");
    }
    let home = std::env::temp_dir().join(format!("fp-technique-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("home");
    let h = home.clone();
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_fastpilot")).configure(move |c| {
        // evalScript only reads button states; every action is a real tap or typed text.
        c.arg("mcp").current_dir(ROOT).env("HOME", &h).env("FASTPILOT_LIVE_DIR", h.join("live")).env("FASTPILOT_ALLOW_EVAL_SCRIPT", "true").env_remove("FASTPILOT_CONFIG").env_remove("FASTPILOT_BLOCK_ACTIONS");
    });
    let client: Client = ().serve(TokioChildProcess::new(cmd).expect("spawn")).await.expect("initialize");
    let res: CallToolResult = client
        .call_tool(CallToolRequestParams::new("session").with_arguments(json!({ "action": "start", "id": "t", "platform": "web", "target": "examples/demo/tickets.html" }).as_object().cloned().unwrap_or_default()))
        .await
        .expect("session");
    assert_ne!(res.is_error, Some(true), "{res:?}");

    // tickets.html: age 18–65, tickets 1–10. Bug: 10 tickets is refused.
    let reserved = |a: u32, q: u32| format!("Reserved: {q} ticket{} for a guest aged {a}.", if q == 1 { "" } else { "s" });
    for (age, qty, expect) in [
        ("17", "2", "Age must be 18 to 65".to_owned()),
        ("18", "2", reserved(18, 2)),
        ("65", "2", reserved(65, 2)),
        ("66", "2", "Age must be 18 to 65".to_owned()),
        ("30", "0", "Tickets must be 1 to 10".to_owned()),
        ("30", "1", reserved(30, 1)),
        ("30", "9", reserved(30, 9)),
        ("30", "10", "Tickets must be 1 to 10".to_owned()), // planted bug: should be reserved
        ("30", "11", "Tickets must be 1 to 10".to_owned()),
        ("30", "2.5", "Enter the number of tickets as a whole number".to_owned()),
        ("abc", "2", "Enter your age as a whole number".to_owned()),
    ] {
        ok(&client, &format!("- openUrl: examples/demo/tickets.html\n- inputText: {{ into: Age, text: \"{age}\" }}\n- inputText: {{ into: Tickets, text: \"{qty}\" }}\n- tapOn: Reserve tickets\n- assertVisible: {}\n", serde_json::to_string(&expect).expect("text"))).await;
    }

    // shipping.html: all 8 rules. Bug: member + express + $50 or more costs $0.00 (should be $10.00).
    for (member, total, express, cost) in [
        (true, "50.00", false, "0.00"),
        (true, "49.99", false, "5.00"),
        (false, "120", false, "5.00"),
        (false, "10", false, "5.00"),
        (true, "50", true, "0.00"), // planted bug: should be 10.00
        (true, "49.99", true, "10.00"),
        (false, "80", true, "15.00"),
        (false, "5", true, "15.00"),
    ] {
        let mut steps = "- openUrl: examples/demo/shipping.html\n".to_owned();
        if member {
            steps.push_str("- tapOn: I am a member\n");
        }
        steps.push_str(&format!("- inputText: {{ into: \"Order total ($)\", text: \"{total}\" }}\n"));
        if express {
            steps.push_str("- tapOn: Express\n");
        }
        steps.push_str(&format!("- tapOn: Show shipping cost\n- assertVisible: \"Shipping: ${cost}\"\n"));
        ok(&client, &steps).await;
    }
    ok(&client, "- openUrl: examples/demo/shipping.html\n- inputText: { into: \"Order total ($)\", text: \"fifty\" }\n- tapOn: Show shipping cost\n- assertVisible: \"Enter the order total, e.g. 49.99\"\n").await;

    // orders.html: allowed moves work, forbidden ones are disabled. Bug: Shipped can be cancelled.
    let buttons = "['paid','ship','deliver','cancel'].filter(id => !document.getElementById(id).disabled).join(',') + '|' + document.getElementById('status').textContent";
    ok(&client, "- openUrl: examples/demo/orders.html\n").await;
    assert_eq!(eval(&client, buttons).await, "paid,cancel|Created");
    ok(&client, "- tapOn: Mark as paid\n- assertVisible: Paid\n").await;
    assert_eq!(eval(&client, buttons).await, "ship,cancel|Paid");
    ok(&client, "- tapOn: Ship\n").await;
    assert_eq!(eval(&client, buttons).await, "deliver,cancel|Shipped", "planted bug: cancel stays enabled on Shipped");
    ok(&client, "- tapOn: Cancel order\n").await;
    assert_eq!(eval(&client, buttons).await, "|Cancelled", "planted bug: a shipped order gets cancelled");
    ok(&client, "- tapOn: Start a new order\n- tapOn: Mark as paid\n- tapOn: Ship\n- tapOn: Mark as delivered\n").await;
    assert_eq!(eval(&client, buttons).await, "|Delivered");
    ok(&client, "- tapOn: Start a new order\n- tapOn: Cancel order\n").await;
    assert_eq!(eval(&client, buttons).await, "|Cancelled");
    ok(&client, "- tapOn: Start a new order\n- tapOn: Mark as paid\n- tapOn: Cancel order\n").await;
    assert_eq!(eval(&client, buttons).await, "|Cancelled");
    let history = eval(&client, "[...document.querySelectorAll('#history li')].map(l => l.textContent).join('>')").await;
    assert_eq!(history, "Created>Paid>Cancelled");

    // None of the practice pages' buttons is stopped by the built-in limits (no Pay, Checkout, ...).
    for name in ["Reserve tickets", "Show shipping cost", "Mark as paid", "Ship", "Mark as delivered", "Cancel order", "Start a new order"] {
        assert!(fastpilot_core::policy::block_match(name, &fastpilot_core::policy::expand_blocked(None)).is_none(), "{name}");
    }
    client.cancel().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(home);
}
