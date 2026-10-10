// POST /api/feedback: a report from the desktop, filed as a GitHub issue in the private
// telemetry repo with GH_TELEMETRY_TOKEN. Body: { kind, title, body, sig }: kind is "feedback"
// (typed by a person), "error" or "panic" (sent by the desktop). A report with a `sig` (an
// error's signature, a feedback's id) files one open issue, titled `[kind] title (sig)`, and is
// deduped against it: first against what this instance filed, then the repo's newest open
// issues (read from the repo, so a minute-old issue is there), then a search (whose index lags,
// so it is only for older ones). If a lookup fails, the report gets 503 and the desktop sends it
// again later, rather than filing a second issue. What is left: two instances filing the same
// new sig in the same moment can each file one. The desktop holds no secrets, so reports carry
// none. Every report is also logged; with no token it gets 503, and the desktop keeps it and
// retries later.

const REPO = process.env.TELEMETRY_REPO || 'compusophy/computehub-telemetry';
const KINDS = ['feedback', 'error', 'panic'];
const MAX_BODY = 32 << 10;
const WINDOW_MS = 3600e3;
const PER_IP = 20;
const PER_INSTANCE = 300;
const hits = new Map();
// `${kind} ${sig}` -> the promise of [status, json] of filing it, while it is filed and after,
// at most FILED of them (oldest go first).
const filed = new Map();
const FILED = 1000;

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

// The open issue filed for `sig` as `kind`: { url, number }, null if there is none, or the
// HTTP status of a lookup that failed.
async function find(kind, sig) {
  const ours = (i) => i.title.startsWith(`[${kind}] `) && i.title.endsWith(` (${sig})`);
  const recent = await github(`/repos/${REPO}/issues?state=open&per_page=100`);
  if (!recent.ok) return recent.status;
  let hit = (await recent.json()).find(ours);
  if (!hit) {
    const q = encodeURIComponent(`repo:${REPO} is:issue is:open in:title ${sig}`);
    const found = await github(`/search/issues?q=${q}&per_page=5`);
    if (!found.ok) return found.status;
    hit = (await found.json()).items?.find(ours);
  }
  return hit ? { url: hit.html_url, number: hit.number } : null;
}

// Files a report, or finds the issue it was filed as: [status, json].
async function file(kind, title, sig, body, host) {
  try {
    if (sig) {
      const hit = await find(kind, sig);
      if (typeof hit === 'number') return [503, { error: `github lookup ${hit}, try later` }];
      if (hit) return [200, { ...hit, deduped: true }];
    }
    const issue = {
      title: `[${kind}] ${title}${sig ? ` (${sig})` : ''}`,
      body: `${body}\n\n---\n*Sent from compusophyOS at ${host}.*`,
      labels: [kind],
    };
    const init = { method: 'POST', body: JSON.stringify(issue) };
    const made = await github(`/repos/${REPO}/issues`, init);
    if (!made.ok) return [502, { error: `github ${made.status}` }];
    const { html_url: url, number } = await made.json();
    return [201, { url, number }];
  } catch {
    return [502, { error: "couldn't reach github" }];
  }
}

// The pages the OS is mounted in as a cartridge (os::cartridge: secretspace's Battlestation),
// whose OS calls this function from their own origin: compusophy's own sites, nothing else.
// Each api/*.mjs ships alone, so each holds this list and the two functions below.
const FRIENDS = [
  'https://compusophy.com',
  'https://secretspace.compusophy.com',
  'https://secretspace-seven.vercel.app',
];

// Whether the caller may: this site's own page (Origin, and Sec-Fetch-Site when a browser sends
// it), or a friend's, whose browser is then told it may read the answer (CORS).
function caller(req, res) {
  const host = req.headers['x-forwarded-host'] || req.headers.host;
  const origin = req.headers.origin;
  const site = req.headers['sec-fetch-site'];
  if (origin === `https://${host}`) return site === undefined || site === 'same-origin';
  if (!FRIENDS.includes(origin)) return false;
  res.setHeader('access-control-allow-origin', origin);
  res.setHeader('vary', 'origin');
  return true;
}

// A friend's browser asks first (a preflight) before a POST it would not send unasked.
function preflight(req, res) {
  const ok = caller(req, res);
  if (ok) {
    res.setHeader('access-control-allow-methods', 'POST');
    res.setHeader('access-control-allow-headers', 'content-type');
    res.setHeader('access-control-max-age', '86400');
  }
  res.statusCode = ok ? 204 : 403;
  res.end();
}

export default async function handler(req, res) {
  if (req.method === 'OPTIONS') return preflight(req, res);
  if (req.method !== 'POST') return reply(res, 405, { error: 'POST only' });
  if (!caller(req, res)) return reply(res, 403, { error: 'this site and its friends only' });
  // The page it came from (a friend's, for an OS mounted there), for the issue's footer.
  const host = String(req.headers.origin).replace('https://', '');
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
  // The same report again, while it is filed or after: the first one's answer.
  const key = `${kind} ${sig}`;
  let job = sig ? filed.get(key) : undefined;
  const again = job !== undefined;
  if (!again) {
    job = file(kind, title, sig, body, host);
    if (sig) {
      if (filed.size >= FILED) filed.delete(filed.keys().next().value);
      filed.set(key, job);
    }
  }
  const [status, json] = await job;
  if (status >= 300) {
    if (filed.get(key) === job) filed.delete(key);
    return reply(res, status, json);
  }
  return again ? reply(res, 200, { ...json, deduped: true }) : reply(res, status, json);
}
