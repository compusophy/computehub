//! The free AI's day ledger (`api/ai.mjs`, the proxy every make's requests go through), run under
//! Node as Vercel runs it: a reply the client leaves before its usage is booked at least what the
//! gateway bills for it, so one client never spends past its share of the day.

use std::io::Write;
use std::process::{Command, Stdio};

/// Runs the real handler for one client, back to back, a minute apart: each request answered by
/// the stream on stdin in parts of 61 bytes (lines and chars split), the client leaving where the
/// stream ends (before its usage), until a request is refused. Prints how many went through and
/// the refusal's status.
const DRIVER: &str = r#"
import { EventEmitter } from 'node:events';
import { pathToFileURL } from 'node:url';
const { default: handler } = await import(pathToFileURL(process.argv[1]).href);
process.env.AI_GATEWAY_API_KEY = 'test';
const parts = [];
for await (const p of process.stdin) parts.push(p);
const sse = Buffer.concat(parts);
const messages = [{ role: 'user', content: 'x'.repeat(9000) }];
const body = Buffer.from(JSON.stringify({ model: 'zai/glm-5.3', max_tokens: 6144, messages }));
let [t, n, status] = [Date.now(), 0, 200];
Date.now = () => t;
while (status === 200) {
  t += 61e3;
  const headers = { origin: 'https://h.test', host: 'h.test', 'x-forwarded-for': '10.0.0.1' };
  const req = { method: 'POST', headers, async *[Symbol.asyncIterator]() { yield body; } };
  const res = Object.assign(new EventEmitter(), { statusCode: 0, headersSent: false });
  res.writableFinished = false;
  res.setHeader = () => {};
  res.write = () => { res.headersSent = true; };
  res.end = () => { res.writableFinished = true; };
  globalThis.fetch = async (_, { signal }) => ({
    ok: true,
    status: 200,
    headers: { get: () => null },
    body: (async function* () {
      for (let i = 0; i < sse.length; i += 61) yield sse.subarray(i, i + 61);
      res.emit('close');
      if (signal.aborted) throw new Error('the client left');
    })() });
  await handler(req, res);
  status = res.statusCode;
  n += status === 200 ? 1 : 0;
}
console.log(n, status);
"#;

/// An SSE event with `text` in its delta's `key`.
fn delta(key: &str, text: &str) -> String {
    let text = text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
    format!("data: {{\"choices\":[{{\"delta\":{{\"{key}\":\"{text}\"}}}}]}}\n\n")
}

#[test]
fn a_client_that_leaves_before_the_usage_never_spends_past_its_share() {
    let proxy = include_str!("../../../api/ai.mjs");
    for line in ["const DAY_BUDGET = 40 / 30;", "const CLIENT_SHARE = 1 / 2;"] {
        assert!(proxy.contains(line), "{line}");
    }
    // As Baseten streams GLM 5.3: about 3 tokens an event (Fireworks sends one), 2.7 chars a
    // token. 700 events of thinking and 200 of a program: 8,500 chars, 3,150 tokens, billed
    // with 2,400 of its 2,500 tokens in cached: $0.0001400 + $0.000624 + $0.01386 = $0.014624.
    let mut sse = String::new();
    for _ in 0..700 {
        sse += &delta("reasoning", "thinking.");
    }
    for _ in 0..200 {
        sse += &delta("content", "label \"\u{2192}\";\n");
    }
    let billed = 14_624u64;
    let share = 40_000_000 / 30 / 2;
    let api = concat!(env!("CARGO_MANIFEST_DIR"), "/../../api/ai.mjs");
    let node = Command::new("node")
        .args(["--input-type=module", "-e", DRIVER, api])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn();
    let mut node = node.expect("node runs the free AI's proxy, as Vercel does: install Node");
    node.stdin.take().unwrap().write_all(sse.as_bytes()).unwrap();
    let out = node.wait_with_output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    let (n, status) = said.trim().split_once(' ').expect("the driver's count");
    let n: u64 = n.parse().unwrap();
    // Refused for the day's budget, never for the request windows, after spending at most the
    // client's share as billed, and not booked so far over that it gets less than half of it.
    assert_eq!(status, "402", "{said}");
    assert!(n * billed <= share && n * billed * 2 > share, "{n} requests: {said}");
}
