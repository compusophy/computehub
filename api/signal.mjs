// POST /api/signal: pairing for the mesh. Two tabs swap WebRTC descriptions here once, by a
// short code, then talk tab to tab; nothing else passes through. The body is lines of text:
//
//   host\n<offer>          keeps the offer under a new code; answers the code
//   meet\n<box>\n<offer>   keeps the offer under a mailbox two linked devices name (24 hex
//                          digits from a secret only they hold), replacing the last; the rest
//                          below works with a mailbox as with a code
//   join\n<code>           answers the offer kept under the code (404: none, or not here yet)
//   answer\n<code>\n<sdp>  keeps the answer to the code's offer
//   poll\n<code>           answers the answer, once (204: none yet; 404: no such code)
//
// Codes live in this instance's own memory for TTL_MS, then are forgotten, as is an answer once
// read: nothing is stored anywhere else. Another instance knows none of them, so a tab that misses
// (404) tries again, which mostly reaches this one. Guards, all best effort: same origin (Origin,
// and Sec-Fetch-Site when a browser sends it), requests per client a minute, a cap on codes kept
// and on a body's size. A description holds the tab's addresses, which is why it lives briefly.

const TTL_MS = 180e3;
const MAX_BODY = 32 << 10;
const MAX_CODES = 2000;
const PER_MINUTE = 120;
// Code letters: no 0/O, 1/I/L, 2/Z, 5/S, 8/B.
const ALPHABET = '34679ACDEFGHJKMNPQRTUVWXY';
const CODE_LEN = 5;
const MAILBOX = /^[0-9A-F]{24}$/;
const codes = new Map();
const hits = new Map();

// Whether `key` may make one more request this minute (counting it if so).
function allow(key, now) {
  if (hits.size > 10000) {
    for (const [k, ts] of hits) if (now - ts[ts.length - 1] > 60e3) hits.delete(k);
  }
  const recent = (hits.get(key) || []).filter((t) => now - t < 60e3);
  hits.set(key, recent);
  if (recent.length >= PER_MINUTE) return false;
  recent.push(now);
  return true;
}

function forget(now) {
  for (const [c, e] of codes) if (now - e.t > TTL_MS) codes.delete(c);
}

function newCode() {
  for (let i = 0; i < 20; i++) {
    let c = '';
    const bytes = crypto.getRandomValues(new Uint8Array(CODE_LEN));
    for (const b of bytes) c += ALPHABET[b % ALPHABET.length];
    if (!codes.has(c)) return c;
  }
  return null;
}

function send(res, status, text = '') {
  res.statusCode = status;
  res.setHeader('content-type', 'text/plain; charset=utf-8');
  res.setHeader('cache-control', 'no-store');
  res.end(text);
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

// Whether `sdp` looks like a session description, and nothing else.
function description(sdp) {
  return typeof sdp === 'string' && sdp.startsWith('v=0') && sdp.length <= MAX_BODY;
}

export default async function handler(req, res) {
  if (req.method !== 'POST') return send(res, 405, 'POST only');
  const host = req.headers['x-forwarded-host'] || req.headers.host;
  const site = req.headers['sec-fetch-site'];
  if (req.headers.origin !== `https://${host}` || (site !== undefined && site !== 'same-origin')) {
    return send(res, 403, 'same origin only');
  }
  const now = Date.now();
  const who = String(req.headers['x-forwarded-for'] || '').split(',')[0].trim() || 'unknown';
  if (!allow(who, now)) return send(res, 429, 'rate limited');
  const text = await readBody(req, MAX_BODY);
  if (text === null) return send(res, 413, 'too long');
  forget(now);
  const nl = text.indexOf('\n');
  const op = nl < 0 ? text : text.slice(0, nl);
  const rest = nl < 0 ? '' : text.slice(nl + 1);
  const [code, ...more] = rest.split('\n');
  const entry = codes.get(code.trim().toUpperCase());
  switch (op) {
    case 'host': {
      if (!description(rest)) return send(res, 400, 'not a description');
      if (codes.size >= MAX_CODES) return send(res, 503, 'busy');
      const c = newCode();
      if (!c) return send(res, 503, 'busy');
      codes.set(c, { t: now, offer: rest, answer: null });
      return send(res, 200, c);
    }
    case 'meet': {
      const box = code.trim().toUpperCase();
      const sdp = more.join('\n');
      if (!MAILBOX.test(box)) return send(res, 400, 'not a mailbox');
      if (!description(sdp)) return send(res, 400, 'not a description');
      if (!codes.has(box) && codes.size >= MAX_CODES) return send(res, 503, 'busy');
      codes.set(box, { t: now, offer: sdp, answer: null });
      return send(res, 200);
    }
    case 'join':
      return entry && !entry.answer ? send(res, 200, entry.offer) : send(res, 404, 'no such code');
    case 'answer': {
      const sdp = more.join('\n');
      if (!entry || entry.answer) return send(res, 404, 'no such code');
      if (!description(sdp)) return send(res, 400, 'not a description');
      entry.answer = sdp;
      return send(res, 200);
    }
    case 'poll':
      if (!entry) return send(res, 404, 'no such code');
      if (!entry.answer) return send(res, 204);
      codes.delete(code.trim().toUpperCase());
      return send(res, 200, entry.answer);
    default:
      return send(res, 400, 'host, meet, join, answer or poll');
  }
}
