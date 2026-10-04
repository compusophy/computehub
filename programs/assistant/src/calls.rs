//! A streamed chat completion that may call tools, read as it arrives: the text, and the
//! `delta.tool_calls` collected by their `index` (`id` and `name` from their first chunk, the
//! `arguments` appended), whether a provider sends a call whole in one chunk or in pieces. A call
//! whose arguments went past [`MAX_ARGS`], or were still coming when the reply ran out of room,
//! is marked [`Call::cut`], so the model hears that rather than that they do not parse.
//! [`crate::json::Stream`] stays as Studio needs it; this is the agent's.

use crate::json::Json;

/// The most calls one reply makes, and bytes of a call's arguments or of the text.
pub const MAX_CALLS: usize = 8;
pub const MAX_ARGS: usize = 8 << 10;
const MAX_TEXT: usize = 16 << 10;
const MAX_OTHER: usize = 16 << 10;

/// One tool call: the provider's id for it, the tool's name, its arguments (JSON text), and
/// whether they were cut off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Call {
    pub id: String,
    pub name: String,
    pub args: String,
    pub cut: bool,
}

/// A reply as read so far.
#[derive(Debug, Default)]
pub struct Calls {
    rest: Vec<u8>,
    other: String,
    pub text: String,
    pub calls: Vec<Call>,
    /// Why it ended, once a chunk said; the service's error; tokens in and out.
    pub finish: String,
    pub error: String,
    pub usage: Option<(u64, u64)>,
}

impl Calls {
    /// Reads `data`: complete lines only, so a chunk may split a line or a character.
    pub fn feed(&mut self, data: &[u8]) {
        self.rest.extend_from_slice(data);
        if let Some(n) = self.rest.iter().rposition(|b| *b == b'\n') {
            let lines: Vec<u8> = self.rest.drain(..=n).collect();
            String::from_utf8_lossy(&lines).lines().for_each(|l| self.line(l));
        }
    }

    /// The body ended: the last line, an error body's message if no chunk said one, and the last
    /// call cut if the reply ran out of room before its arguments closed.
    pub fn end(&mut self) {
        let rest = std::mem::take(&mut self.rest);
        self.line(&String::from_utf8_lossy(&rest));
        let said = Json::parse(&self.other).and_then(|v| message(&v).map(String::from));
        if let (true, Some(m)) = (self.error.is_empty(), said) {
            self.error = m;
        }
        if let (true, Some(c)) = (self.finish == "length", self.calls.last_mut()) {
            c.cut |= !matches!(Json::parse(&c.args), Some(Json::Obj(_)));
        }
    }

    fn line(&mut self, line: &str) {
        let line = line.trim_end_matches('\r');
        let Some(data) = line.strip_prefix("data:").map(str::trim_start) else {
            if !line.starts_with(':') && self.other.len() + line.len() <= MAX_OTHER {
                self.other.push_str(line);
            }
            return;
        };
        let Some(v) = Json::parse(data) else { return };
        let choice = v.get("choices").and_then(|c| c.at(0));
        if let Some(f) = choice.and_then(|c| c.get("finish_reason")?.text()) {
            self.finish = f.into();
        }
        let delta = choice.and_then(|c| c.get("delta"));
        if let Some(t) = delta.and_then(|d| d.get("content")?.text()) {
            if self.text.len() + t.len() <= MAX_TEXT {
                self.text.push_str(t);
            }
        }
        let tools = delta.and_then(|d| d.get("tool_calls"));
        for tc in (0..).map_while(|k| tools?.at(k)) {
            self.call(tc);
        }
        if let Some(m) = message(&v) {
            self.error = m.into();
        }
        let usage = v.get("usage");
        let tokens = |k| usage?.get(k)?.text()?.parse::<u64>().ok();
        if let (Some(i), Some(o)) = (tokens("prompt_tokens"), tokens("completion_tokens")) {
            self.usage = Some((i, o));
        }
    }

    /// A tool call delta: to the call at its index, else a new call if it has an id, else the
    /// last one.
    fn call(&mut self, tc: &Json) {
        let text = |v: Option<&Json>| v.and_then(Json::text).map(str::to_string);
        let f = tc.get("function");
        let (id, name) = (text(tc.get("id")), text(f.and_then(|f| f.get("name"))));
        let args = f.and_then(|f| f.get("arguments"));
        let index = text(tc.get("index")).and_then(|i| i.parse::<usize>().ok());
        let new = if id.is_some() { self.calls.len() } else { self.calls.len().saturating_sub(1) };
        let at = index.unwrap_or(new);
        if at >= MAX_CALLS {
            return;
        }
        while self.calls.len() <= at {
            self.calls.push(Call::default());
        }
        let c = &mut self.calls[at];
        c.id = id.filter(|i| !i.is_empty()).unwrap_or(std::mem::take(&mut c.id));
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            c.name = n;
        }
        // Arguments come as text to append; a provider may send them as an object.
        let args = match args {
            Some(Json::Str(s)) => s.clone(),
            Some(v @ Json::Obj(_)) => encode(v),
            _ => String::new(),
        };
        if c.args.len() + args.len() <= MAX_ARGS {
            c.args.push_str(&args);
        } else {
            c.cut = true;
        }
    }
}

/// `error.message`, or `error` when it is a string.
fn message(v: &Json) -> Option<&str> {
    let e = v.get("error")?;
    e.get("message").unwrap_or(e).text()
}

/// `v` as JSON text.
pub fn encode(v: &Json) -> String {
    use crate::json::quote;
    match v {
        Json::Null => "null".into(),
        Json::Bool(b) => b.to_string(),
        Json::Num(n) => n.clone(),
        Json::Str(s) => quote(s),
        Json::Arr(a) => ["[", &a.iter().map(encode).collect::<Vec<_>>().join(","), "]"].concat(),
        Json::Obj(m) => {
            let members: Vec<String> =
                m.iter().map(|(k, v)| [quote(k), encode(v)].join(":")).collect();
            ["{", &members.join(","), "}"].concat()
        }
    }
}
