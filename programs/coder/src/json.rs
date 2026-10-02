//! Just enough JSON: a strict reader for the AI service's chunks and a string
//! quoter for the requests; and [`Stream`], the SSE body read as it arrives, with what it cost
//! ([`Usage`]).

/// A JSON value; a number keeps its text.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// The deepest nesting [`Json::parse`] reads.
const MAX_DEPTH: usize = 64;
/// Each one-letter escape, then the char it stands for.
const ESCAPES: &[u8] = b"n\nt\tr\rb\x08f\x0c\"\"\\\\//";

impl Json {
    /// Exactly one value with optional whitespace around it, or `None`.
    pub fn parse(s: &str) -> Option<Json> {
        let mut p = Parser { s: s.as_bytes(), i: 0 };
        let v = p.value(0)?;
        p.ws();
        (p.i == p.s.len()).then_some(v)
    }

    /// An object's member.
    pub fn get(&self, key: &str) -> Option<&Json> {
        if let Json::Obj(m) = self { m.iter().find(|m| m.0 == key).map(|m| &m.1) } else { None }
    }

    /// An array's element.
    pub fn at(&self, i: usize) -> Option<&Json> {
        if let Json::Arr(a) = self { a.get(i) } else { None }
    }

    /// A string's or number's text.
    pub fn text(&self) -> Option<&str> {
        if let Json::Str(s) | Json::Num(s) = self { Some(s) } else { None }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while matches!(self.s.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    /// Whether the next byte after whitespace is `b`, taking it if so.
    fn eat(&mut self, b: u8) -> bool {
        self.ws();
        let ok = self.s.get(self.i) == Some(&b);
        self.i += usize::from(ok);
        ok
    }

    /// The items of an array or object, its opening byte taken, up to `close`.
    fn items(&mut self, close: u8, mut item: impl FnMut(&mut Self) -> Option<()>) -> Option<()> {
        let mut first = true;
        while !self.eat(close) {
            (first || self.eat(b',')).then(|| item(self))??;
            first = false;
        }
        Some(())
    }

    fn value(&mut self, depth: usize) -> Option<Json> {
        self.ws();
        let word = |p: &mut Self, w: &str, v| {
            p.s[p.i..].starts_with(w.as_bytes()).then(|| p.i += w.len()).map(|_| v)
        };
        let (deeper, start) = (Some(depth + 1).filter(|d| *d <= MAX_DEPTH), self.i);
        Some(match *self.s.get(self.i)? {
            b'{' => {
                self.i += 1;
                let mut m = Vec::new();
                self.items(b'}', |p| {
                    let k = p.string().filter(|_| p.eat(b':'))?;
                    p.value(deeper?).map(|v| m.push((k, v)))
                })?;
                Json::Obj(m)
            }
            b'[' => {
                self.i += 1;
                let mut a = Vec::new();
                self.items(b']', |p| p.value(deeper?).map(|v| a.push(v)))?;
                Json::Arr(a)
            }
            b'"' => Json::Str(self.string()?),
            b't' => word(self, "true", Json::Bool(true))?,
            b'f' => word(self, "false", Json::Bool(false))?,
            b'n' => word(self, "null", Json::Null)?,
            b'-' | b'0'..=b'9' => {
                let n = self.s[self.i..].iter().take_while(|b| b"+-.eE0123456789".contains(b));
                self.i += n.count();
                Json::Num(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
            }
            _ => return None,
        })
    }

    /// A string at the cursor; a lone surrogate escape becomes U+FFFD.
    fn string(&mut self) -> Option<String> {
        self.eat(b'"').then_some(())?;
        let mut out = String::new();
        loop {
            // A run of plain bytes ends at an ASCII byte, so it is whole UTF-8.
            let rest = self.s.get(self.i..)?;
            let n = rest.iter().position(|b| matches!(b, b'"' | b'\\'))?;
            out.push_str(core::str::from_utf8(&rest[..n]).ok()?);
            self.i += n + 2;
            let Some(&e) = rest.get(n + 1).filter(|_| rest[n] == b'\\') else {
                self.i -= 1;
                return (rest[n] == b'"').then_some(out);
            };
            let pair = ESCAPES.chunks(2).find(|p| p[0] == e).map(|p| char::from(p[1]));
            out.push(if e == b'u' { self.unicode()? } else { pair? });
        }
    }

    /// The char of a `\u` escape (its `\u` taken), joining a surrogate pair.
    fn unicode(&mut self) -> Option<char> {
        let hex = |p: &mut Self| {
            let h = p.s.get(p.i..p.i + 4).filter(|h| h.iter().all(u8::is_ascii_hexdigit))?;
            p.i += 4;
            u32::from_str_radix(core::str::from_utf8(h).ok()?, 16).ok()
        };
        let mut c = hex(self)?;
        if (0xD800..0xDC00).contains(&c) && self.s[self.i..].starts_with(b"\\u") {
            self.i += 2;
            let lo = hex(self)?.checked_sub(0xDC00).filter(|lo| *lo < 0x400);
            c = lo.map_or(0xFFFD, |lo| 0x10000 + ((c - 0xD800) << 10) + lo);
        }
        Some(char::from_u32(c).unwrap_or('\u{FFFD}'))
    }
}

/// `s` as a JSON string.
pub fn quote(s: &str) -> String {
    let mut out = String::new();
    put(&mut out, s);
    out
}

/// Appends `s` to `out` as a JSON string.
pub fn put(out: &mut String, s: &str) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push('"');
    for c in s.chars() {
        match c {
            '"' | '\\' => out.extend(['\\', c]),
            c if c < ' ' => {
                let hex = |n: u32| char::from(HEX[n as usize & 15]);
                out.extend(['\\', 'u', '0', '0', hex(u32::from(c) >> 4), hex(u32::from(c))]);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// What a reply cost, as its usage chunk said: tokens in (of them, read from the provider's
/// cache) and out (of them, reasoning: thinking a provider spent, streamed or not).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: u32,
    pub cached: u32,
    pub output: u32,
    pub reasoning: u32,
}

/// A server-sent-events chat-completion body, read in chunks of any size:
/// complete lines only, so a chunk may split a line or a UTF-8 sequence.
#[derive(Debug, Default)]
pub struct Stream {
    /// The bytes after the last complete line.
    rest: Vec<u8>,
    /// Lines that are not SSE (an error's JSON body), up to [`MAX_OTHER`] bytes.
    other: String,
    /// The AI service's error message, if it sent one.
    pub error: String,
    /// What the reply cost, once the usage chunk came (a cancelled reply never has it).
    pub usage: Option<Usage>,
    /// The chars of reasoning deltas so far: the model thinking, never part of the text.
    pub thought: usize,
    /// Why the reply ended, once a chunk said: `stop`, or `length` when it ran out of room.
    pub finish: String,
}

const MAX_OTHER: usize = 16 * 1024;

impl Stream {
    /// Reads `data`, appending each content delta to `text` while it is under `max` bytes and
    /// counting reasoning deltas in [`Stream::thought`].
    pub fn feed(&mut self, data: &[u8], text: &mut String, max: usize) {
        self.rest.extend_from_slice(data);
        if let Some(n) = self.rest.iter().rposition(|b| *b == b'\n') {
            let lines: Vec<u8> = self.rest.drain(..=n).collect();
            for line in String::from_utf8_lossy(&lines).lines() {
                self.line(line, text, max);
            }
        }
    }

    /// The body ended: reads the last line, and an error body if no chunk said one.
    pub fn end(&mut self, text: &mut String, max: usize) {
        let rest = std::mem::take(&mut self.rest);
        self.line(&String::from_utf8_lossy(&rest), text, max);
        if let (true, Some(m)) = (self.error.is_empty(), error(self.other.as_bytes())) {
            self.error = m;
        }
    }

    /// One line. A chunk is read for the members it may hold (a choice's `finish_reason`, a
    /// delta's `content`, `reasoning` or `reasoning_content`, an `error`, the `usage`), by key,
    /// rather than whole: a key in a string is escaped (`\"content\":`), so it never matches.
    fn line(&mut self, line: &str, text: &mut String, max: usize) {
        let line = line.trim_end_matches('\r');
        let Some(data) = line.strip_prefix("data:").map(str::trim_start) else {
            if !line.starts_with(':') && self.other.len() + line.len() <= MAX_OTHER {
                self.other.push_str(line);
            }
            return;
        };
        let b = data.as_bytes();
        if let Some(f) = string(b, "finish_reason") {
            self.finish = f;
        }
        if let Some(d) = string(b, "content").filter(|_| text.len() < max) {
            text.push_str(&d);
        }
        let thought = string(b, "reasoning").or_else(|| string(b, "reasoning_content"));
        self.thought += thought.map_or(0, |t| t.chars().count());
        if let Some(m) = error(b) {
            self.error = m;
        }
        // The last usage: the gateway's own metadata comes earlier in the chunk.
        let Some(at) = (0..b.len()).rev().find(|&i| b[i..].starts_with(b"\"usage\":{")) else {
            return;
        };
        let n = |k| number(&b[at..], k).and_then(whole);
        if let (Some(input), Some(output)) = (n("prompt_tokens"), n("completion_tokens")) {
            let (cached, reasoning) = (n("cached_tokens"), n("reasoning_tokens"));
            let (cached, reasoning) = (cached.unwrap_or(0), reasoning.unwrap_or(0));
            self.usage = Some(Usage { input, cached, output, reasoning });
        }
    }
}

/// Where the value of the first `"key":` in `b` starts.
fn value(b: &[u8], key: &str) -> Option<usize> {
    let pat = ["\"", key, "\":"].concat();
    let pat = pat.as_bytes();
    b.windows(pat.len()).position(|w| w == pat).map(|i| i + pat.len())
}

/// The string value of the first `"key":` in `b`, if it is one.
fn string(b: &[u8], key: &str) -> Option<String> {
    Parser { s: b, i: value(b, key)? }.string()
}

/// The text of the number value of the first `"key":` in `b`, if it is one.
fn number<'a>(b: &'a [u8], key: &str) -> Option<&'a str> {
    let at = value(b, key)?;
    let rest = &b[at..];
    let start = rest.iter().position(|c| !c.is_ascii_whitespace())?;
    // Signs and exponents are taken too, so that they fail to read rather than read short.
    let n = rest[start..].iter().take_while(|c| b"+-.eE0123456789".contains(c)).count();
    core::str::from_utf8(&rest[start..start + n]).ok().filter(|s| !s.is_empty())
}

/// Where the value of the outermost object's member `"key":` starts in `b` (`b` from that
/// object's `{`): a member of a nested object is not it.
fn member(b: &[u8], key: &str) -> Option<usize> {
    let pat = ["\"", key, "\""].concat();
    let (mut depth, mut text, mut i) = (0, false, 0);
    while i < b.len() {
        match b[i] {
            b'\\' if text => i += 1,
            b'"' if !text && depth == 1 && b[i..].starts_with(pat.as_bytes()) => {
                let mut p = Parser { s: b, i: i + pat.len() };
                if p.eat(b':') {
                    return Some(p.i);
                }
                text = true;
            }
            b'"' => text = !text,
            b'{' | b'[' if !text => depth += 1,
            b'}' | b']' if !text => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

/// The chunk's own `error.message`, or `error` when it is a string; never one nested in it (the
/// gateway's metadata lists each provider it tried, with the errors of those that failed).
fn error(b: &[u8]) -> Option<String> {
    let mut p = Parser { s: b, i: member(b, "error")? };
    p.ws();
    match b.get(p.i)? {
        b'{' => Parser { s: b, i: p.i + member(&b[p.i..], "message")? }.string(),
        _ => p.string(),
    }
}

/// `s`, digits only (at most 9), as a number; `None` for anything else.
fn whole(s: &str) -> Option<u32> {
    let ok = !s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit());
    ok.then(|| s.bytes().fold(0, |n, b| n * 10 + u32::from(b - b'0')))
}
