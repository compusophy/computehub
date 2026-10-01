// POST /api/feedback: a report from the desktop, filed as a GitHub issue in
// the private telemetry repo with GH_TELEMETRY_TOKEN. Body: { kind, title,
// body, sig }: kind is "feedback" (typed by a person), "error" or "panic"
// (sent by the desktop, deduped by `sig` against open issues). The desktop
// holds no secrets, so reports carry none. Every report is also logged; with
// no token it gets 503, and the desktop keeps it and retries later.

const REPO = process.env.TELEMETRY_REPO || 'compusophy/computehub-telemetry';
const KINDS = ['feedback', 'error', 'panic'];
const MAX_BODY = 32 << 10;
const WINDOW_MS = 3600e3;
const PER_IP = 20;
const PER_INSTANCE = 300;
const hits = new Map();

function allow(key, limit, now) {
  const recent = (hits.get(key) || []).filter((t) => now - t < WINDOW_MS);
  hits.set(key, recent);
  if (recent.length >= limit) return false;
  recent.push(now);
  return true;
}

function reply(res, status, json) {
  res.statusCode = status;
  res.setHeader('content-type', 'application/json');
  res.end(JSON.stringify(json));
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

async function github(path, init) {
  const headers = {
    authorization: `Bearer ${process.env.GH_TELEMETRY_TOKEN}`,
    accept: 'application/vnd.github+json',
    'content-type': 'application/json',
    'user-agent': 'compusophy-telemetry',
  };
  return fetch(`https://api.github.com${path}`, { ...init, headers });
}

export default async function handler(req, res) {
  if (req.method !== 'POST') return reply(res, 405, { error: 'POST only' });
  const host = req.headers['x-forwarded-host'] || req.headers.host;
  if (req.headers.origin !== `https://${host}`) return reply(res, 403, { error: 'same origin only' });
  const now = Date.now();
  const ip = String(req.headers['x-forwarded-for'] || '').split(',')[0].trim() || 'unknown';
  if (!allow(ip, PER_IP, now) || !allow('*', PER_INSTANCE, now)) {
    return reply(res, 429, { error: 'too many reports' });
  }
  let r;
  try {
    r = JSON.parse(await readBody(req, MAX_BODY));
  } catch {
    return reply(res, 400, { error: 'bad or oversized json' });
  }
  const kind = KINDS.includes(r && r.kind) ? r.kind : 'feedback';
  const clean = (s, n) => String(s ?? '').trim().slice(0, n);
  const title = clean(r && r.title, 120) || kind;
  const sig = clean(r && r.sig, 32).replace(/[^\w-]/g, '');
  const body = clean(r && r.body, MAX_BODY);
  console.log(JSON.stringify({ report: { kind, title, sig, body } }));
  if (!process.env.GH_TELEMETRY_TOKEN) return reply(res, 503, { error: 'inbox not configured' });
  try {
    if (sig && kind !== 'feedback') {
      const q = encodeURIComponent(`repo:${REPO} is:issue is:open in:title ${sig}`);
      const found = await github(`/search/issues?q=${q}&per_page=5`);
      const hit = found.ok && (await found.json()).items?.find((i) => i.title.includes(`(${sig})`));
      if (hit) return reply(res, 200, { url: hit.html_url, number: hit.number, deduped: true });
    }
    const issue = {
      title: `[${kind}] ${title}${sig ? ` (${sig})` : ''}`,
      body: `${body}\n\n---\n*Sent from compusophyOS at ${host}.*`,
      labels: [kind],
    };
    const made = await github(`/repos/${REPO}/issues`, { method: 'POST', body: JSON.stringify(issue) });
    if (!made.ok) return reply(res, 502, { error: `github ${made.status}` });
    const { html_url: url, number } = await made.json();
    return reply(res, 201, { url, number });
  } catch {
    return reply(res, 502, { error: "couldn't reach github" });
  }
}
