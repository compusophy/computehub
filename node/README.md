# computehub-node

The node is a small native program that gives compusophyOS terminals a real
shell on your machine. The OS runs in a browser tab. A browser tab cannot
start processes, so the node starts them: it listens on loopback, and each
authenticated WebSocket connection gets its own shell (PowerShell, bash, zsh,
and so on) in its own pseudo-terminal. Anything that runs in a normal terminal
runs there too, including full-screen programs like `vim` and the `claude` CLI.

It is the first piece of computehub that runs outside the browser. It is its
own Cargo workspace (`node/`, package `computehub-node`) because it needs
native libraries: `portable-pty` (ConPTY on Windows, openpty on Unix),
`tungstenite` (a synchronous WebSocket server) and `getrandom` (the token).
It has no async runtime; it uses std threads. The zero-dependency rule
applies to the OS crates under `crates/`, not to this workspace.

## Run it

```sh
cd node && cargo run --release
```

It prints something like this:

```text
computehub node 0.1.0
listening on 127.0.0.1:7878 (loopback only)
shell: windows/powershell   max sessions: 8
allowed origins:
  https://computehub-sigma.vercel.app

pair a compusophyOS terminal by opening this link (it holds the token):
https://computehub-sigma.vercel.app/#node=7878&token=<64 hex digits>
```

Options (pass them after `--` when you use `cargo run`, for example
`cargo run --release -- --port 9000`). For the local preview, start it with
`--url http://localhost:8080`:

| option | default | meaning |
|---|---|---|
| `--port <n>` | 7878 | port on 127.0.0.1; 0 picks a free one |
| `--allow-origin <origin>` | none | also accept pages from this origin, e.g. `http://localhost:3000`; repeatable |
| `--url <page url>` | `https://computehub-sigma.vercel.app` | the page the pairing link opens; its origin is the only one allowed by default |
| `--max-sessions <n>` | 8 | concurrent shells (1 to 256); extra connections get ERROR `busy` |
| `-- <program> [args]` | see below | the shell to run instead of the default |
| `--help`, `--version` | | |

The default shell is `pwsh.exe -NoLogo` on Windows when PowerShell 7 is on
`PATH`, otherwise `powershell.exe -NoLogo`; elsewhere it is `$SHELL`, or
`/bin/sh`. Each shell starts at 80 x 24 in your home directory with
`TERM=xterm-256color` and `COLORTERM=truecolor`, and otherwise inherits the
node's environment. So start the node from a terminal where your tools are on
`PATH`. (If you start it from inside a Claude Code session, the `claude` CLI
sees that session's `CLAUDECODE` variable and refuses to nest. Start the node
from an ordinary terminal instead.)

## Pairing

1. Start the node. It makes a fresh random token every time it starts and
   prints the pairing link: `<url>/#node=<port>&token=<64 hex digits>`.
2. Open the link in the browser where compusophyOS runs. The token is in the
   URL fragment (after `#`), and browsers never send the fragment to a server.
   So the token goes from your terminal to your browser tab and nowhere else.
   The page should remove it from the address bar once it has read it.
3. The OS opens `ws://127.0.0.1:<port>/` and sends HELLO with the token. It
   gets READY back, then a live shell.
4. Stopping the node (Ctrl+C) ends every session and invalidates the token.
   To pair again, restart the node and open the new link.

## Security model

The node hands out a full shell running as you, so the token is as sensitive
as your password. It is a pairing secret for this run only and is never
written to disk.

- **Loopback only.** It binds 127.0.0.1 and nothing else, so other machines
  cannot connect.
- **Origin allowlist.** Browsers attach an `Origin` header that pages cannot
  forge. The handshake is refused with HTTP 403 when the header is missing or
  not on the list. This stops every other website you have open, including
  DNS-rebinding attacks, before it can send a single byte to a shell. The
  list holds the `--url` page's origin and each `--allow-origin`, nothing
  else: a local dev server is trusted only when you name it.
- **Token.** Every connection must send HELLO with the 32-byte token within 5
  seconds of connecting; the handshake counts against those 5 seconds,
  however slowly its bytes arrive. The node compares the token in constant
  time. A wrong token is answered after a 1 second delay with close code
  4401. The delay holds only that connection. There is no lockout: wrong
  tokens cost nothing to send, so a shared lockout would let any local
  program lock you out. With a 256-bit token, guessing is hopeless.
- **Limits.** At most 16 connections may be waiting to authenticate. A 17th
  evicts the one that has waited longest, so connections another program
  parks cannot keep yours out (it is through in milliseconds). At most
  `--max-sessions` shells may run. Client messages are capped at 1 MiB.
- **Nothing identifying goes out.** READY names only the OS and the shell's
  file name (`windows/powershell`), never a hostname, user name or path. The
  banner prints no hostname or user name. Error messages sent to clients are
  generic; details go to the node's stderr. The shell's own output is passed
  through unchanged, and it can include paths, for example in the prompt.
- **What it does not defend against:** other programs already running as you
  on this machine. They can read your files and processes without the node.
  And a local program that floods the port with connections can crowd it,
  as it could any loopback service.

When a client disconnects, the node kills its shell and frees its PTY. On
Windows, closing the pseudoconsole also ends every process attached to it.

## Protocol

Every message is one binary WebSocket message. The first byte is the opcode,
and integers are little-endian. A text message is a protocol error.

| op | name | direction | payload |
|---|---|---|---|
| 0x01 | HELLO | client to node | `[u8 version = 1][32-byte token]`; must be first, within 5 s |
| 0x02 | READY | node to client | `[u8 version = 1][u16 cols][u16 rows][UTF-8 "<os>/<shell>"]` |
| 0x03 | DATA | both ways | client to node: bytes for the shell's input. Node to client: shell output, at most 64 KiB per message |
| 0x04 | RESIZE | client to node | `[u16 cols][u16 rows]`, clamped to 1..=1000 x 1..=500 |
| 0x05 | EXIT | node to client | `[i32 exit code]`, then the node closes (code 1000) |
| 0x06 | ERROR | node to client | UTF-8 message, then the node closes |

Close codes: 1000 after EXIT, 4401 for a wrong token, 1008 for a missing or
late HELLO, 1002 after a protocol ERROR, 1013 after `busy`, 1011 if the shell
could not start.

DATA is a byte stream. A UTF-8 character or an escape sequence can be split
across two messages, so the terminal must parse across message boundaries.

`src/proto.rs` is the reference encoder and decoder for both directions.

### Notes for terminal authors

- On Windows the shell runs under ConPTY, which emits VT sequences for an
  xterm-like terminal. It also turns on win32-input-mode (`CSI ? 9001 h`)
  and focus reporting (`CSI ? 1004 h`); a terminal that does not support
  them can ignore both and send plain VT input.
- At startup ConPTY asks for the cursor position (`CSI 6 n`) and draws
  nothing until it gets an answer. The node answers that first query itself
  (a new session's cursor is at 1;1) and removes it from the output, so
  clients never see it. Later queries from programs are passed through, and
  the terminal should answer them as usual.
- Window titles (OSC 0/2) come from the shell. On Windows they often hold the
  program's path.

## Develop

```sh
cd node
cargo fmt
cargo test                               # unit tests + in-process end to end
cargo clippy --all-targets -- -D warnings
cargo +1.85 build                        # the MSRV
```

The end-to-end tests start a server in-process on an ephemeral port. They
connect with `tungstenite` and run a real command in a real PTY
(`cmd.exe /c echo hello` on Windows, `/bin/sh -c "echo hello"` elsewhere).
They also check the wrong-token close, that wrong tokens never lock out the
right one, eviction of parked connections, the handshake deadline, the Origin
refusal, protocol errors and `busy`.
