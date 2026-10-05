// POST /api/ai: the free AI. Takes an OpenAI-style chat-completions body and sends the Vercel AI
// Gateway a new one built from it (an allowed model, its messages, the tools the overlay's agent
// may call, bounded output, bounded reasoning and the providers that keep to it (see below), a
// temperature; each checked and rebuilt field by field, nothing else it sent goes on), with
// this project's own credentials (its OIDC token, or AI_GATEWAY_API_KEY when set), and streams
// the answer back, so no visitor needs a key and the browser never holds one.
//
// Guards, all best effort: same origin (Origin, and Sec-Fetch-Site when a browser sends it);
// request windows per client (an IPv4 address or an IPv6 /64) and per instance; and a day's
// spend per instance, of which one client may use a share, counted from the usage the gateway
// reports. A script can forge Origin and leave out Sec-Fetch-Site, and an instance shares no
// state with others, so these slow one client down and nothing more. The hard ceiling is a
// budget on the gateway: one on this project (`vercel ai-gateway budgets set project <name>
// --limit 40`) covers the OIDC calls; a key's own budget covers only calls made with that key.

const GATEWAY = 'https://ai-gateway.vercel.sh/v1/chat/completions';
// The models allowed, the first the default, with their list prices in dollars per million
// tokens in and out.
const MODELS = { 'zai/glm-5.3': [1.4, 4.4], 'zai/glm-5.3-flash': [0.15, 0.5] };
// The providers asked first, as the gateway names them: those measured keeping to a thinking
// budget (see below). Another joins once measured doing so. A model's fallback there, and the
// statuses (busy, at capacity, down) that send a request on to the next of TRIES.
const PROVIDERS = ['fireworks'];
const FALLBACK = { 'zai/glm-5.3': ['zai/glm-5.3-flash'] };
const TRIES = [{ only: PROVIDERS }, { only: PROVIDERS, wait: 1200 }, { order: PROVIDERS }];
const BUSY = [429, 500, 502, 503, 504];
const ROLES = ['system', 'user', 'assistant', 'tool'];
// Tools (the agent that uses the desktop): how many, their JSON, the calls a reply may make, a
// call's arguments, the output a request with tools may ask for; names and call ids.
const MAX_MESSAGES = 64, MAX_TOOLS = 16, MAX_TOOLS_JSON = 16 << 10;
const MAX_CALLS = 8, MAX_ARGS = 8 << 10, TOOL_OUT = 2048;
const NAME = /^[A-Za-z_][A-Za-z0-9_]{0,63}$/, CALL_ID = /^[\w.-]{1,64}$/;
const MAX_BODY = 96 << 10;
const MAX_TOKENS = 8192;
const WINDOW_MS = 3600e3;
// An agent's task makes a request a step: 120 an hour, and 30 a minute stops a runaway loop.
const PER_CLIENT = 120;
const PER_MINUTE = 30;
const PER_INSTANCE = 400;
// An instance's spend a day (the month's $40 over 30 days), and the part of it one client gets.
const DAY_MS = 24 * WINDOW_MS;
const DAY_BUDGET = 40 / 30;
const CLIENT_SHARE = 1 / 2;
const hits = new Map();
// This instance's requests in the last day, oldest first: { t, who, usd }.
const ledger = [];

// Whether `key` may make one more request in a window of `ms` (counting it if so).
function allow(key, limit, now, ms = WINDOW_MS) {
  if (hits.size > 10000) {
    for (const [k, ts] of hits) if (now - ts[ts.length - 1] > WINDOW_MS) hits.delete(k);
  }
  const recent = (hits.get(key) || []).filter((t) => now - t < ms);
  hits.set(key, recent);
  if (recent.length >= limit) return false;
  recent.push(now);
  return true;
}

// Dollars this instance spent in the last day: all of it, or `who`'s.
function spent(now, who) {
  while (ledger.length && now - ledger[0].t >= DAY_MS) ledger.shift();
  let usd = 0;
  for (const e of ledger) if (who === undefined || e.who === who) usd += e.usd;
  return usd;
}

// Who a request comes from: its IPv4 address, or its IPv6 /64 (one host often holds a whole
// /64, so a full address would give it a fresh window per address).
function client(req) {
  const ip = String(req.headers['x-forwarded-for'] || '').split(',')[0].trim();
  if (!ip.includes(':')) return ip || 'unknown';
  const v4 = /^::ffff:(\d+\.\d+\.\d+\.\d+)$/i.exec(ip);
  if (v4) return v4[1];
  const [head, tail] = ip.split('%')[0].split('::');
  const a = head ? head.split(':') : [];
  const b = tail ? tail.split(':') : [];
  const zeros = tail === undefined ? [] : Array(Math.max(0, 8 - a.length - b.length)).fill('0');
  const prefix = [...a, ...zeros, ...b].slice(0, 4);
  return `${prefix.map((g) => (parseInt(g, 16) || 0).toString(16)).join(':')}::/64`;
}

const text = (v, max) => typeof v === 'string' && v.length <= max;
const object = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
// A string that `re` matches (test() would take undefined for "undefined").
const is = (re, v) => typeof v === 'string' && re.test(v);

// A message rebuilt from `m`, or null if it is not one: system and user text; tool text
// answering a call id; assistant text, or null or '' with 1 to MAX_CALLS tool calls.
function message(m) {
  if (!object(m) || !ROLES.includes(m.role)) return null;
  const { role, content } = m;
  if (role === 'tool') {
    return text(content, Infinity) && is(CALL_ID, m.tool_call_id)
      ? { role, tool_call_id: m.tool_call_id, content } : null;
  }
  const calls = role === 'assistant' && Array.isArray(m.tool_calls) ? m.tool_calls : [];
  if (calls.length > MAX_CALLS || (role !== 'assistant' && m.tool_calls !== undefined)) return null;
  const tool_calls = [];
  for (const c of calls) {
    const f = object(c) && object(c.function) ? c.function : {};
    const ok = object(c) && is(CALL_ID, c.id) && c.type === 'function' && is(NAME, f.name);
    if (!ok || !text(f.arguments, MAX_ARGS)) return null;
    tool_calls.push({ id: c.id, type: 'function', function: { name: f.name, arguments: f.arguments } });
  }
  if (tool_calls.length) {
    return content === null || text(content, Infinity) ? { role, content: content || null, tool_calls } : null;
  }
  return text(content, Infinity) ? { role, content } : null;
}

// The tools rebuilt from `tools`, or null if they are not: each a function with a name, a short
// description and a plain object of parameters.
function toolsOf(tools) {
  if (!Array.isArray(tools) || tools.length > MAX_TOOLS) return null;
  if (JSON.stringify(tools).length > MAX_TOOLS_JSON) return null;
  const out = [];
  for (const t of tools) {
    const f = object(t) && object(t.function) ? t.function : {};
    if (!object(t) || t.type !== 'function' || !is(NAME, f.name) || !text(f.description, 1024) || !object(f.parameters)) {
      return null;
    }
    out.push({ type: 'function', function: { name: f.name, description: f.description, parameters: f.parameters } });
  }
  return out;
}

function fail(res, status, error) {
  res.statusCode = status;
  res.setHeader('content-type', 'application/json');
  res.end(JSON.stringify({ error: { message: error } }));
}

// Counts into `seen` what `text`, the next of a stream, carried: its events, and the chars of text
// their deltas held (content, reasoning, tool calls' arguments), a line at a time (a part may end
// mid-line; the rest waits in `seen.line`).
function streamed(seen, text) {
  const lines = (seen.line + text).split('\n');
  seen.line = lines.pop();
  for (const line of lines) {
    if (!line.startsWith('data:')) continue;
    seen.events += 1;
    let j;
    try {
      j = JSON.parse(line.slice(5));
    } catch {
      continue;
    }
    for (const c of object(j) && Array.isArray(j.choices) ? j.choices : []) {
      const d = object(c) && object(c.delta) ? c.delta : {};
      for (const v of [d.content, d.reasoning, d.reasoning_content]) {
        if (typeof v === 'string') seen.chars += v.length;
      }
      for (const t of Array.isArray(d.tool_calls) ? d.tool_calls : []) {
        const f = object(t) && object(t.function) ? t.function : {};
        if (typeof f.arguments === 'string') seen.chars += f.arguments.length;
      }
    }
  }
}

async function readBody(req, max) {
  const parts = [];
  let n = 0;
  for await (const part of req) {
    n += part.length;
    if (n > max) return null;
    parts.push(part);
  }
  return Buffer.concat(parts).toString('utf8');
}

export default async function handler(req, res) {
  if (req.method !== 'POST') return fail(res, 405, 'POST only');
  const host = req.headers['x-forwarded-host'] || req.headers.host;
  const site = req.headers['sec-fetch-site'];
  if (req.headers.origin !== `https://${host}` || (site !== undefined && site !== 'same-origin')) {
    return fail(res, 403, 'same origin only');
  }
  const now = Date.now();
  const who = client(req);
  const ok = allow(who, PER_CLIENT, now) && allow(`${who} m`, PER_MINUTE, now, 60e3);
  if (!ok || !allow('*', PER_INSTANCE, now)) {
    return fail(res, 429, 'rate limited');
  }
  const token = process.env.AI_GATEWAY_API_KEY || req.headers['x-vercel-oidc-token'];
  if (!token) return fail(res, 503, 'not configured');
  const text = await readBody(req, MAX_BODY);
  if (text === null) return fail(res, 413, `the request is over ${MAX_BODY >> 10} KB`);
  let body;
  try {
    body = JSON.parse(text);
  } catch {
    return fail(res, 400, 'bad json');
  }
  const given = Array.isArray(body && body.messages) ? body.messages : [];
  const messages = given.map(message);
  if (messages.includes(null) || messages.length > MAX_MESSAGES) {
    return fail(res, 400, 'each message must be a system, user, assistant or tool message');
  }
  if (!messages.length) return fail(res, 400, 'no messages');
  const tools = body.tools === undefined ? [] : toolsOf(body.tools);
  const choice = body.tool_choice === undefined ? 'auto' : body.tool_choice;
  if (!tools || !['auto', 'none'].includes(choice)) {
    return fail(res, 400, "tools must be functions, and tool_choice 'auto' or 'none'");
  }
  // The desktop puts the model chosen in Settings first in every body, so a program's own, after
  // it, wins: JSON.parse keeps a key's last value.
  const model = Object.hasOwn(MODELS, body.model) ? body.model : Object.keys(MODELS)[0];
  const asked = Math.floor(Number(body.max_tokens));
  // Thinking costs time and tokens, and GLM 5.3 thinks at length however it is asked not to
  // (unseen, up to the whole output: measured, told off 3,166 tokens, at a low effort 8,192 of
  // 8,192 and no program). Only a token budget bounds it, and only where the provider keeps to
  // one: Fireworks does, for GLM 5.3 and Flash (a budget of 1,024 held it to 1,037, in 37
  // replies; one of 256 to 1,036, so none holds under about 1,024); Baseten, DigitalOcean and
  // Alibaba do not (2,000 of 2,000; 7,333 of 8,192 and no program), and a reply there that
  // thinks its whole room makes nothing (17 of 20 eval requests fell back on 2026-10-02, and
  // tetris, 2048 and minesweeper got no program). The gateway picks a provider itself (by
  // weight, by affinity, after a failure), so it is asked in TRIES: PROVIDERS only, the model
  // then its FALLBACK there; again after a breath; only then any provider, since an answer
  // that may think past its budget beats none (Studio re-asks when thinking eats a reply's
  // room). The budget is 1,024 tokens, or a program's own (at most 4,096); an effort or
  // thinking off, which bound nothing, are not sent.
  const r = (body && body.reasoning) || {};
  const budget = Math.floor(Number(r.max_tokens));
  const reasoning = { max_tokens: budget >= 1 ? Math.min(budget, 4096) : 1024 };
  const out = {
    model,
    messages,
    stream: true,
    stream_options: { include_usage: true },
    max_tokens: asked >= 1 ? Math.min(asked, MAX_TOKENS) : 4096,
    reasoning,
  };
  if (tools.length) {
    Object.assign(out, { tools, tool_choice: choice, max_tokens: Math.min(out.max_tokens, TOOL_OUT) });
  }
  const t = body.temperature;
  if (typeof t === 'number' && t >= 0 && t <= 2) out.temperature = t;
  // What it could cost (a token per 3 chars in, every output token used), held until the usage
  // says what it did cost: its own `cost` (the gateway's price, cached tokens at theirs), else its
  // tokens at list price. A request the client stopped (Studio stops one once its program is in)
  // never gets the usage: it costs the input and the text streamed, a token per 2.5 of its chars
  // (GLM 5.3 writes 2.64 to 4.2 a token, measured on 29 replies, so this books a little over),
  // or per event if more, never more than was held. Not a token per event: Fireworks sends about
  // one an event, but Baseten and Runware 2.7 to 5.2, which booked a stopped reply at 38 to 48%.
  const [priceIn, priceOut] = MODELS[model];
  const chars = JSON.stringify(messages).length + (tools.length ? JSON.stringify(tools).length : 0);
  const entry = { t: now, who, usd: ((chars / 3) * priceIn + out.max_tokens * priceOut) / 1e6 };
  const over = (usd, cap) => usd + entry.usd > cap;
  if (over(spent(now), DAY_BUDGET) || over(spent(now, who), DAY_BUDGET * CLIENT_SHARE)) {
    return fail(res, 402, "today's budget is spent, try again later");
  }
  ledger.push(entry);
  const abort = new AbortController();
  res.on('close', () => res.writableFinished || abort.abort());
  let [up, tail] = [null, ''];
  const [seen, utf8] = [{ events: 0, chars: 0, line: '' }, new TextDecoder()];
  // The last usage the stream reported: tokens in and out, and its own cost (NaN if not said).
  const used = () => {
    const usage = tail.slice(tail.lastIndexOf('"usage"'));
    const num = (k) => Number((new RegExp(`"${k}":\\s*([\\d.]+)`).exec(usage) || [])[1]);
    return [num('prompt_tokens'), num('completion_tokens'), num('cost')];
  };
  try {
    for (const { wait, ...gateway } of TRIES) {
      if (wait) await new Promise((done) => setTimeout(done, wait));
      if (gateway.only) gateway.models = FALLBACK[model];
      out.providerOptions = { gateway };
      up = await fetch(GATEWAY, {
        method: 'POST',
        headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
        body: JSON.stringify(out),
        signal: abort.signal,
      });
      if (up.ok || !BUSY.includes(up.status)) break;
      await up.text().catch(() => '');
    }
    // A refusal costs nothing.
    if (!up.ok) entry.usd = 0;
    res.statusCode = up.status;
    res.setHeader('content-type', up.headers.get('content-type') || 'text/event-stream');
    res.setHeader('cache-control', 'no-store');
    for await (const part of up.body) {
      res.write(part);
      const text = utf8.decode(part, { stream: true });
      streamed(seen, text);
      tail = (tail + text).slice(-4096);
    }
    // The receipt, the stream's last line (an SSE comment, which readers skip): its tokens and
    // what it cost in millionths of a dollar, which the desktop adds up for Activity.
    const [i, o, cost] = used();
    if (up.ok && i >= 0 && o >= 0) {
      const microusd = Math.round(cost >= 0 ? cost * 1e6 : i * priceIn + o * priceOut);
      res.write(`\n: receipt in=${i} out=${o} microusd=${microusd}\n\n`);
    }
    res.end();
  } catch {
    // Never reached (or only refused), it cost nothing.
    if (!up || !up.ok) entry.usd = 0;
    if (!res.headersSent) return fail(res, 502, "couldn't reach the AI");
    // Cut off mid-answer: say so, on a line of its own, so it never reads as a whole answer.
    res.end('\n\ndata: {"error":{"message":"the answer was cut off"}}\n\n');
  }
  const [i, o, cost] = used();
  if (cost >= 0) entry.usd = cost;
  else if (i >= 0 && o >= 0) entry.usd = (i * priceIn + o * priceOut) / 1e6;
  else if (up && up.ok) {
    const tokens = Math.min(out.max_tokens, Math.max(seen.events, seen.chars / 2.5));
    entry.usd = Math.min(entry.usd, ((chars / 3) * priceIn + tokens * priceOut) / 1e6);
  }
}
