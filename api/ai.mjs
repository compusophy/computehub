// POST /api/ai: the free AI. Takes an OpenAI-style chat-completions body and sends the Vercel AI
// Gateway a new one built from it (an allowed model, its text messages, bounded output,
// reasoning off unless it asks for a low effort, a temperature; nothing else it sent goes on), with this project's own
// credentials (its OIDC token, or AI_GATEWAY_API_KEY when set), and streams the answer back, so
// no visitor needs a key and the browser never holds one.
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
const ROLES = ['system', 'user', 'assistant'];
const MAX_BODY = 96 << 10;
const MAX_TOKENS = 8192;
const WINDOW_MS = 3600e3;
const PER_CLIENT = 20;
const PER_INSTANCE = 400;
// An instance's spend a day (the month's $40 over 30 days), and the part of it one client gets.
const DAY_MS = 24 * WINDOW_MS;
const DAY_BUDGET = 40 / 30;
const CLIENT_SHARE = 1 / 2;
const hits = new Map();
// This instance's requests in the last day, oldest first: { t, who, usd }.
const ledger = [];

// Whether `key` may make one more request this window (counting it if so).
function allow(key, limit, now) {
  if (hits.size > 10000) {
    for (const [k, ts] of hits) if (now - ts[ts.length - 1] > WINDOW_MS) hits.delete(k);
  }
  const recent = (hits.get(key) || []).filter((t) => now - t < WINDOW_MS);
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

function fail(res, status, error) {
  res.statusCode = status;
  res.setHeader('content-type', 'application/json');
  res.end(JSON.stringify({ error: { message: error } }));
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
  if (!allow(who, PER_CLIENT, now) || !allow('*', PER_INSTANCE, now)) {
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
  const messages = [];
  for (const m of Array.isArray(body && body.messages) ? body.messages : []) {
    if (!m || !ROLES.includes(m.role) || typeof m.content !== 'string') {
      return fail(res, 400, 'each message must be a system, user or assistant text');
    }
    messages.push({ role: m.role, content: m.content });
  }
  if (!messages.length) return fail(res, 400, 'no messages');
  const model = Object.hasOwn(MODELS, body.model) ? body.model : Object.keys(MODELS)[0];
  const asked = Math.floor(Number(body.max_tokens));
  // Thinking costs time and tokens: off, unless the program asks for a low effort (Studio does,
  // for better code).
  const low = body.reasoning && body.reasoning.effort === 'low';
  const out = {
    model,
    messages,
    stream: true,
    stream_options: { include_usage: true },
    max_tokens: asked >= 1 ? Math.min(asked, MAX_TOKENS) : 4096,
    reasoning: low ? { effort: 'low' } : { enabled: false },
  };
  const t = body.temperature;
  if (typeof t === 'number' && t >= 0 && t <= 2) out.temperature = t;
  // What it could cost (a token per 3 chars in, every output token used), held until the usage
  // says what it did cost.
  const [priceIn, priceOut] = MODELS[model];
  const chars = messages.reduce((n, m) => n + m.content.length, 0);
  const entry = { t: now, who, usd: ((chars / 3) * priceIn + out.max_tokens * priceOut) / 1e6 };
  const over = (usd, cap) => usd + entry.usd > cap;
  if (over(spent(now), DAY_BUDGET) || over(spent(now, who), DAY_BUDGET * CLIENT_SHARE)) {
    return fail(res, 402, "today's budget is spent, try again later");
  }
  ledger.push(entry);
  const abort = new AbortController();
  res.on('close', () => res.writableFinished || abort.abort());
  let [up, tail] = [null, ''];
  try {
    up = await fetch(GATEWAY, {
      method: 'POST',
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: JSON.stringify(out),
      signal: abort.signal,
    });
    // A refusal costs nothing.
    if (!up.ok) entry.usd = 0;
    res.statusCode = up.status;
    res.setHeader('content-type', up.headers.get('content-type') || 'text/event-stream');
    res.setHeader('cache-control', 'no-store');
    for await (const part of up.body) {
      res.write(part);
      tail = (tail + Buffer.from(part).toString('latin1')).slice(-4096);
    }
    res.end();
  } catch {
    // Never reached, it cost nothing.
    if (!up) entry.usd = 0;
    if (!res.headersSent) return fail(res, 502, "couldn't reach the AI");
    // Cut off mid-answer: say so, on a line of its own, so it never reads as a whole answer.
    res.end('\n\ndata: {"error":{"message":"the answer was cut off"}}\n\n');
  }
  const usage = tail.slice(tail.lastIndexOf('"usage"'));
  const tokens = (k) => Number((new RegExp(`"${k}":\\s*(\\d+)`).exec(usage) || [])[1]);
  const [i, o] = [tokens('prompt_tokens'), tokens('completion_tokens')];
  if (i >= 0 && o >= 0) entry.usd = (i * priceIn + o * priceOut) / 1e6;
}
