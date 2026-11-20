//! B2: `fastpilot run` driven by an OpenAI-compatible chat-completions API instead of Claude Code
//! (opt-in: `provider` setting; Claude Code stays the default). The same tools, the same
//! `finish` verdict, the same step budget (`max_steps`) and time limit (`test_timeout`) as a Claude
//! Code session: only the model loop differs.
//!
//! Presets set the base URL: openai, openrouter, groq, ollama (local, no key); `custom` takes
//! `provider_base_url`. The API key comes from `FASTPILOT_PROVIDER_API_KEY` or the key store
//! (`fastpilot config set-key <provider>`), is sent only as a bearer header, and never appears in
//! logs, reports or a command line.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// One configured OpenAI-compatible provider.
pub struct Provider {
    pub name: String,
    pub base_url: String,
    pub model: String,
    key: Option<String>,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the key.
        f.debug_struct("Provider").field("name", &self.name).field("base_url", &self.base_url).field("model", &self.model).field("key", &self.key.as_ref().map(|_| "***")).finish()
    }
}

use crate::keystore::KeyFrom;

impl Provider {
    /// The `provider` setting, when it is not Claude Code: `None` for claude (or unset).
    pub fn configured() -> Option<Result<Self, String>> {
        let var = |k: &str| fastpilot_core::config::var(k).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let name = var("FASTPILOT_PROVIDER").filter(|p| p != "claude")?;
        Some(Self::resolve(&name, var("FASTPILOT_PROVIDER_BASE_URL"), var("FASTPILOT_PROVIDER_MODEL"), crate::keystore::get_bound(&name)))
    }

    /// `key`: the key and where it comes from.
    fn resolve(name: &str, base_url: Option<String>, model: Option<String>, key: Option<(String, KeyFrom)>) -> Result<Self, String> {
        use fastpilot_core::config::{origin, preset_base};
        if !fastpilot_core::config::PROVIDERS.iter().any(|(p, _)| *p == name) {
            return Err(format!("unknown provider `{name}`"));
        }
        let preset = preset_base(name);
        let base_url = match (name, base_url) {
            // A hosted preset talks to its own server only: another URL here is a leftover (or a
            // mistake) and would get this provider's key.
            ("openai" | "openrouter" | "groq", Some(url)) if origin(&url) != preset.and_then(origin) => {
                return Err(format!(
                    "provider `{name}` only talks to {}, but provider_base_url is {url}: `fastpilot config unset provider_base_url`, or use provider custom for another server",
                    preset.unwrap_or_default()
                ));
            }
            (_, url) => url.or_else(|| preset.map(str::to_owned)),
        }
        .ok_or_else(|| format!("provider `{name}` needs a base URL: `fastpilot config set provider_base_url https://…/v1`"))?;
        let model = model.ok_or_else(|| format!("provider `{name}` needs a model: `fastpilot config set provider_model <model id>`"))?;
        // A stored key goes only to the server it was saved for. One saved before keys were bound
        // (no origin) goes only to its hosted preset's server; for any other server it must be
        // saved again.
        if let Some((_, KeyFrom::Store(saved))) = &key {
            let Some(saved_for) = saved.clone().or_else(|| match name {
                "openai" | "openrouter" | "groq" => preset.and_then(origin),
                _ => None,
            }) else {
                return Err(format!(
                    "the `{name}` API key was saved by an older fastpilot without the server it is for, so it is not sent anywhere: run `fastpilot config set-key {name}` again"
                ));
            };
            if origin(&base_url).as_deref() != Some(saved_for.as_str()) {
                return Err(format!(
                    "the `{name}` API key was saved for {saved_for}, not for {}: it is not sent there; run `fastpilot config set-key {name}` again for this server, or set provider_base_url back",
                    origin(&base_url).unwrap_or(base_url.clone())
                ));
            }
        }
        let key = key.map(|(k, _)| k);
        // A local Ollama needs no key; every hosted API does.
        if key.is_none() && name != "ollama" && fastpilot_core::policy::remote_host(&base_url).is_some() {
            return Err(format!("no API key for `{name}`: run `fastpilot config set-key {name}` (or set FASTPILOT_PROVIDER_API_KEY)"));
        }
        Ok(Self { name: name.to_owned(), base_url: base_url.trim_end_matches('/').to_owned(), model, key })
    }

    /// Redirects are not followed (Security Low 3): the request body (screen contents) would go
    /// to wherever the bound server points.
    fn client() -> Result<reqwest::Client, String> {
        reqwest::Client::builder().timeout(Duration::from_secs(180)).redirect(reqwest::redirect::Policy::none()).build().map_err(|e| e.to_string())
    }

    /// One chat-completions call. Errors never contain the key.
    async fn complete(&self, client: &reqwest::Client, body: &Value) -> Result<Value, String> {
        let mut req = client.post(format!("{}/chat/completions", self.base_url)).header("User-Agent", concat!("fastpilot/", env!("CARGO_PKG_VERSION"))).json(body);
        if let Some(k) = &self.key {
            req = req.bearer_auth(k);
        }
        let mask = |s: String| match &self.key {
            Some(k) => mask_key(&s, k),
            None => s,
        };
        let resp = req.send().await.map_err(|e| {
            // Nobody listening: say where, and for Ollama how to start it.
            if e.is_connect() {
                let hint = if self.name == "ollama" { "; is Ollama running (`ollama serve`) with the model pulled (`ollama pull <model>`)?" } else { "; is the server up and the URL right?" };
                return mask(format!("cannot reach {} at {}{hint}", self.name, self.base_url));
            }
            mask(format!("{}: {e}", self.name))
        })?;
        let status = resp.status();
        if status.is_redirection() {
            let to = resp.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("?").chars().take(200).collect::<String>();
            return Err(mask(format!(
                "{} at {} answered with a redirect (HTTP {}) to {to}: fastpilot does not follow it; set provider_base_url to the final server if you trust it",
                self.name,
                self.base_url,
                status.as_u16()
            )));
        }
        let text = resp.text().await.map_err(|e| mask(format!("{}: {e}", self.name)))?;
        if !status.is_success() {
            let msg = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["error"]["message"].as_str().map(str::to_owned)).unwrap_or_else(|| text.chars().take(300).collect());
            return Err(mask(format!("{} answered HTTP {status}: {msg}", self.name)));
        }
        serde_json::from_str(&text).map_err(|e| format!("{}: not a chat-completions reply: {e}", self.name))
    }
}

/// What one provider session reported (as for a Claude Code session).
pub struct Outcome {
    pub turns: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub failure: Option<String>,
}

/// `text` without `key`, and without the partly masked forms some APIs echo back
/// (`sk-proj********1234`): any word starting with the key's first 6 characters becomes `***`.
pub fn mask_key(text: &str, key: &str) -> String {
    let text = text.replace(key, "***");
    let prefix: String = key.chars().take(6).collect();
    if prefix.chars().count() < 6 {
        return text;
    }
    text.split_inclusive(char::is_whitespace)
        .map(|w| {
            let word = w.trim_end();
            let trimmed = word.trim_start_matches(|c: char| !c.is_alphanumeric());
            if trimmed.starts_with(&prefix) { w.replacen(trimmed.trim_end_matches(['.', ',', ';', ')', '"', '\'']), "***", 1) } else { w.to_owned() }
        })
        .collect()
}

/// The origin a key for `provider` is tied to when it is saved now: the preset's server, or for
/// custom (and a non-default ollama) the configured provider_base_url of that provider.
pub fn key_origin(provider: &str) -> Result<String, String> {
    use fastpilot_core::config::{file_values, origin, preset_base};
    // The settings as they are now (env, then the file), not as this process pinned them at start:
    // `fastpilot ui` saves the provider and its URL, then the key, in one request.
    let now = |name: &str, key: &str| std::env::var(name).ok().or_else(|| file_values().remove(key)).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    let current = now("FASTPILOT_PROVIDER", "provider");
    let base = now("FASTPILOT_PROVIDER_BASE_URL", "provider_base_url");
    let url = match provider {
        "custom" | "ollama" if current.as_deref() == Some(provider) && base.is_some() => base,
        "custom" => {
            return Err("a custom key is tied to its server: `fastpilot config set provider custom` and `provider_base_url` first, then set-key".into());
        }
        p => preset_base(p).map(str::to_owned),
    };
    url.as_deref().and_then(origin).ok_or_else(|| format!("no server known for provider `{provider}`"))
}

/// The test's tools in the OpenAI `tools` format (from the MCP tool list).
pub fn openai_tools(mcp_tools: &[Value]) -> Vec<Value> {
    mcp_tools
        .iter()
        .map(|t| {
            json!({ "type": "function", "function": { "name": t["name"], "description": t["description"], "parameters": t["inputSchema"].clone() } })
        })
        .collect()
}

/// What one test asks of the model.
pub struct Task<'a> {
    pub system: &'a str,
    pub prompt: &'a str,
    /// OpenAI `tools` ([`openai_tools`]).
    pub tools: Vec<Value>,
    pub max_turns: usize,
    /// `test_timeout`: the whole loop, model calls included.
    pub limit: Duration,
}

/// Drive one test: chat with tools until the model calls `finish`, the turns run out or the time
/// limit passes. `call` runs one tool and returns its text and whether it failed; `done` says
/// whether `finish` was called.
pub async fn drive<C, Fut, D>(provider: &Provider, task: Task<'_>, mut call: C, done: D) -> Outcome
where
    C: FnMut(String, Value) -> Fut,
    Fut: std::future::Future<Output = (String, bool)>,
    D: Fn() -> bool,
{
    let mut out = Outcome { turns: 0, input_tokens: 0, output_tokens: 0, failure: None };
    let client = match Provider::client() {
        Ok(c) => c,
        Err(e) => {
            out.failure = Some(e);
            return out;
        }
    };
    let started = Instant::now();
    let Task { system, prompt, tools, max_turns, limit } = task;
    let mut messages = vec![json!({ "role": "system", "content": system }), json!({ "role": "user", "content": prompt })];
    let mut nudged = false;
    while out.turns < max_turns {
        if started.elapsed() >= limit {
            out.failure = Some(format!("stopped: the test took longer than {} min (test_timeout)", limit.as_secs() / 60));
            return out;
        }
        out.turns += 1;
        let body = json!({ "model": provider.model, "messages": messages, "tools": tools, "tool_choice": "auto" });
        let left = limit.saturating_sub(started.elapsed());
        let reply = match tokio::time::timeout(left, provider.complete(&client, &body)).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                out.failure = Some(e);
                return out;
            }
            Err(_) => {
                out.failure = Some(format!("stopped: the test took longer than {} min (test_timeout)", limit.as_secs() / 60));
                return out;
            }
        };
        out.input_tokens += reply["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
        out.output_tokens += reply["usage"]["completion_tokens"].as_u64().unwrap_or(0);
        let msg = reply["choices"][0]["message"].clone();
        let calls = msg["tool_calls"].as_array().cloned().unwrap_or_default();
        messages.push(json!({ "role": "assistant", "content": msg["content"].clone(), "tool_calls": if calls.is_empty() { Value::Null } else { Value::Array(calls.clone()) } }));
        if calls.is_empty() {
            if done() || nudged {
                return out;
            }
            // Text without a tool call: once, ask for the verdict.
            nudged = true;
            messages.push(json!({ "role": "user", "content": "Continue with the tools, and end with the finish tool (status pass or fail, with notes)." }));
            continue;
        }
        for c in calls {
            let name = c["function"]["name"].as_str().unwrap_or("").to_owned();
            let args = c["function"]["arguments"].as_str().and_then(|a| serde_json::from_str::<Value>(a).ok()).unwrap_or_else(|| json!({}));
            let (text, _failed) = call(name, args).await;
            messages.push(json!({ "role": "tool", "tool_call_id": c["id"].clone(), "content": text }));
        }
        if done() {
            return out;
        }
    }
    if !done() {
        out.failure = Some(format!("the model used {max_turns} turns without calling finish"));
    }
    out
}

/// `fastpilot config test-provider`: one tiny request with the configured provider.
pub async fn test_provider() -> Result<String, String> {
    let p = Provider::configured().ok_or("provider-nya claude (Claude Code): tidak ada yang dites di sini; `fastpilot doctor run` mengecek Claude Code")??;
    let client = Provider::client()?;
    let t0 = Instant::now();
    let body = json!({ "model": p.model, "messages": [{ "role": "user", "content": "Reply with the word OK." }], "max_tokens": 5 });
    let reply = p.complete(&client, &body).await?;
    let said = reply["choices"][0]["message"]["content"].as_str().unwrap_or("").trim().chars().take(40).collect::<String>();
    let key = match (&p.key, p.name.as_str()) {
        (Some(_), _) => "key dari penyimpanan key atau FASTPILOT_PROVIDER_API_KEY",
        (None, _) => "tanpa key (lokal)",
    };
    Ok(format!("ok: {} di {} menjawab sebagai {} dalam {} ms ({key}): {said:?}", p.name, p.base_url, p.model, t0.elapsed().as_millis()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_missing_pieces() {
        let key = |k: &str| Some((k.to_owned(), KeyFrom::Env));
        let p = Provider::resolve("openai", None, Some("m".into()), key("sk-x")).expect("openai");
        assert_eq!((p.base_url.as_str(), p.model.as_str()), ("https://api.openai.com/v1", "m"));
        assert!(!format!("{p:?}").contains("sk-x"), "Debug never shows the key");
        let o = Provider::resolve("ollama", None, Some("llama".into()), None).expect("ollama needs no key");
        assert_eq!(o.base_url, "http://127.0.0.1:11434/v1");
        assert!(Provider::resolve("groq", None, Some("m".into()), None).expect_err("hosted needs a key").contains("fastpilot config set-key groq"));
        assert!(Provider::resolve("openrouter", None, None, key("k")).expect_err("model").contains("provider_model"));
        assert!(Provider::resolve("custom", None, Some("m".into()), key("k")).expect_err("base url").contains("provider_base_url"));
        let c = Provider::resolve("custom", Some("http://127.0.0.1:9/v1/".into()), Some("m".into()), None).expect("local custom without key");
        assert_eq!(c.base_url, "http://127.0.0.1:9/v1");
    }

    /// QA / Design (Medium): after `provider custom` with its own server and a key saved for openai,
    /// switching to openai kept the old base URL and sent the openai key there. A hosted preset now
    /// only talks to its own server, and a stored key only goes to the origin it was saved for.
    #[test]
    fn a_key_goes_only_to_the_server_it_was_saved_for() {
        let saved = |k: &str, o: &str| Some((k.to_owned(), KeyFrom::Store(Some(o.to_owned()))));
        let m = || Some("m".to_owned());
        // openai with a leftover custom URL: refused, nothing is sent.
        let e = Provider::resolve("openai", Some("http://127.0.0.1:18767/v1".into()), m(), saved("sk-o", "https://api.openai.com")).expect_err("leftover URL");
        assert!(e.contains("only talks to https://api.openai.com/v1") && e.contains("unset provider_base_url"), "{e}");
        // The same server spelled with a path is fine.
        assert!(Provider::resolve("openai", Some("https://api.openai.com/v1/".into()), m(), saved("sk-o", "https://api.openai.com")).is_ok());
        // A custom key saved for one server is not sent to another one.
        let e = Provider::resolve("custom", Some("https://other.example/v1".into()), m(), saved("sk-c", "https://mine.example")).expect_err("other server");
        assert!(e.contains("saved for https://mine.example, not for https://other.example") && !e.contains("sk-c"), "{e}");
        assert!(Provider::resolve("custom", Some("https://mine.example/v2".into()), m(), saved("sk-c", "https://mine.example")).is_ok());
        // Security Low 2 / QA: a key saved before keys were bound (no origin) went to any
        // provider_base_url. Now only to its hosted preset; a custom server needs `set-key` again.
        let old = |k: &str| Some((k.to_owned(), KeyFrom::Store(None)));
        let e = Provider::resolve("custom", Some("https://any.example/v1".into()), m(), old("sk-old")).expect_err("unbound custom key");
        assert!(e.contains("saved by an older fastpilot") && e.contains("config set-key custom") && !e.contains("sk-old"), "{e}");
        assert!(Provider::resolve("ollama", Some("https://ollama.example/v1".into()), m(), old("sk-old")).is_err(), "remote ollama");
        assert!(Provider::resolve("openai", None, m(), old("sk-old")).is_ok(), "hosted preset: its own server");
        // An env key (no origin) is the person's choice for this run.
        assert!(Provider::resolve("custom", Some("https://other.example/v1".into()), m(), Some(("sk-e".into(), KeyFrom::Env))).is_ok());
    }

    #[test]
    fn provider_errors_never_echo_the_key() {
        let k = "sk-openai-secret-777";
        let echoed = "Incorrect API key provided: sk-opena********-777. You can find your key at https://x.";
        let masked = mask_key(echoed, k);
        assert!(!masked.contains("sk-opena") && !masked.contains("-777") && masked.contains("Incorrect API key provided: ***."), "{masked}");
        assert_eq!(mask_key(&format!("bad {k} here"), k), "bad *** here");
        assert_eq!(mask_key("nothing to hide", "short"), "nothing to hide");
    }

    /// QA (Low): a stopped Ollama gave a bare connection error; now it says where and what to run.
    #[tokio::test]
    async fn an_unreachable_provider_says_where_and_how() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").expect("bind").local_addr().expect("addr").port();
        let p = Provider::resolve("ollama", Some(format!("http://127.0.0.1:{port}/v1")), Some("llama3".into()), None).expect("ollama");
        let e = p.complete(&Provider::client().expect("client"), &json!({ "model": "llama3", "messages": [] })).await.expect_err("nobody listens");
        assert!(e.contains(&format!("cannot reach ollama at http://127.0.0.1:{port}/v1")) && e.contains("ollama serve"), "{e}");
    }


    /// Security Low 3: the provider client followed redirects, so a 307/308 from the bound server
    /// sent the request body (screen contents) on to another host. Now it stops with a message.
    #[tokio::test]
    async fn a_redirect_is_not_followed() {
        use std::io::{BufRead, BufReader, Read, Write};
        let elsewhere = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let to = format!("http://127.0.0.1:{}/v1/chat/completions", elsewhere.local_addr().expect("addr").port());
        elsewhere.set_nonblocking(true).expect("nonblocking");
        let server = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = server.local_addr().expect("addr").port();
        let location = to.clone();
        std::thread::spawn(move || {
            if let Ok((stream, _)) = server.accept() {
                let mut r = BufReader::new(stream.try_clone().expect("clone"));
                let mut len = 0usize;
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
                        break;
                    }
                    if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; len];
                let _ = r.read_exact(&mut body);
                let mut s = stream;
                let _ = write!(s, "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
        });
        let p = Provider::resolve("custom", Some(format!("http://127.0.0.1:{port}/v1")), Some("m".into()), Some(("sk-redirect-123".into(), KeyFrom::Env))).expect("custom");
        let e = p.complete(&Provider::client().expect("client"), &json!({ "model": "m", "messages": [{ "role": "user", "content": "screen" }] })).await.expect_err("redirect");
        assert!(e.contains("redirect (HTTP 307)") && e.contains(&to) && !e.contains("sk-redirect-123"), "{e}");
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(elsewhere.accept().is_err(), "the redirect target got a request");
    }

}
