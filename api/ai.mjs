// POST /api/ai: the free AI. Forwards an OpenAI-style chat-completions body
// to the Vercel AI Gateway with this project's own credentials (its OIDC
// token, or AI_GATEWAY_API_KEY when set) and streams the answer back, so no
// visitor needs a key and the browser never holds one. Guards: same origin,
// an allowed model, bounded request and output, and request windows per IP
// and per instance (best effort: an instance shares no state with others;
// the gateway's budget is the hard ceiling).

const GATEWAY = 'https://ai-gateway.vercel.sh/v1/chat/completions';
const MODELS = ['zai/glm-5.3', 'zai/glm-5.3-flash'];
const MAX_BODY = 96 << 10;
const MAX_TOKENS = 8192;
const WINDOW_MS = 3600e3;
const PER_IP = 40;
const PER_INSTANCE = 800;
const hits = new Map();

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
  if (req.headers.origin !== `https://${host}`) return fail(res, 403, 'same origin only');
  const now = Date.now();
  const ip = String(req.headers['x-forwarded-for'] || '').split(',')[0].trim() || 'unknown';
  if (!allow(ip, PER_IP, now) || !allow('*', PER_INSTANCE, now)) {
    return fail(res, 429, 'rate limited');
  }
  const token = process.env.AI_GATEWAY_API_KEY || req.headers['x-vercel-oidc-token'];
  if (!token) return fail(res, 503, 'not configured');
  const text = await readBody(req, MAX_BODY);
  let body;
  try {
    body = JSON.parse(text);
  } catch {
    return fail(res, 400, text === null ? 'request too large' : 'bad json');
  }
  if (!body || !Array.isArray(body.messages)) return fail(res, 400, 'no messages');
  body.model = MODELS.includes(body.model) ? body.model : MODELS[0];
  body.stream = true;
  body.max_tokens = Math.min(Number(body.max_tokens) || 4096, MAX_TOKENS);
  // Thinking costs time and tokens: off unless the program asks for it.
  if (body.reasoning === undefined) body.reasoning = { enabled: false };
  const abort = new AbortController();
  res.on('close', () => res.writableFinished || abort.abort());
  try {
    const up = await fetch(GATEWAY, {
      method: 'POST',
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: JSON.stringify(body),
      signal: abort.signal,
    });
    res.statusCode = up.status;
    res.setHeader('content-type', up.headers.get('content-type') || 'text/event-stream');
    res.setHeader('cache-control', 'no-store');
    for await (const part of up.body) res.write(part);
    res.end();
  } catch (e) {
    if (!res.headersSent) return fail(res, 502, "couldn't reach the AI");
    res.end();
  }
}
