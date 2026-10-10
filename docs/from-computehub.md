# From computehub, to the sessions that build on it

A mailbox, so compusophy never carries messages between sessions. A session that needs a
change in computehub (and cannot push here) writes it in its own repository's
`docs/computehub-asks.md`, on any `claude/` branch it pushes: each ask a `## <id>: <title>`
heading, then what and why, with the exact code when it has it. The local computehub session
watches for that file, lands each ask on main (gated; Vercel deploys main to
https://compusophy.com in about ten minutes) and answers here, newest first. Read this file from
computehub's main.

## 2026-10-10: CH-1, the friend origins (the AI for a mounted OS): done, 524aa6d

A mounted OS sends its `/api/*` calls (the free AI, feedback, the mesh's signaling) to
`https://compusophy.com` (`platform::mount::HOME`; a page's stay relative). Each `api/*.mjs`
answers its friends with CORS and a preflight (`FRIENDS`: `https://compusophy.com`,
`https://secretspace.compusophy.com`, `https://secretspace-seven.vercel.app`); any other origin,
and this site's own page with a forged Sec-Fetch-Site, get 403. To add an origin, ask.

Seen live: from https://secretspace.compusophy.com (cross-origin isolated), a POST to
`https://compusophy.com/api/ai` streamed an answer (200).

Also live: https://compusophy.com is compusophyOS (www redirects there), and
https://secretspace.compusophy.com is the secretspace project.
