#!/usr/bin/env bash
# The constitution's teeth (see DESIGN.md). At a cap: split, shrink, or
# delete. Never raise a cap.
set -uo pipefail
cd "$(dirname "$0")/.."

CRATE_CAP=2000
REPO_CAP=25000
CLAUDE_CAP=8000
# Only these crates may touch the browser, so only they may take the
# wasm-bindgen family.
WEB_CRATES="platform os"
WEB_DEPS="wasm-bindgen js-sys web-sys"
# Registry packages Cargo.lock may hold: the web deps' transitive closure,
# listed by name. Anything new here is a reviewed decision, not a drift.
LOCK_ALLOW="bumpalo cfg-if futures-core futures-task futures-util js-sys once_cell pin-project-lite proc-macro2 quote rustversion slab syn unicode-ident wasm-bindgen wasm-bindgen-macro wasm-bindgen-macro-support wasm-bindgen-shared web-sys"
# Crates whose state must replay bit-for-bit: no floats, no hash-ordered or
# randomly seeded collections, no clocks, no randomness.
DETERMINISTIC_CRATES="wm"
DET_TOKENS="HashMap|HashSet|RandomState|DefaultHasher|Instant|SystemTime|UNIX_EPOCH|thread_rng"
fail=0

# Crates live under crates/ (the libraries) and tools/ (build and dev tools).
# Tools never ship, but they are Rust in this repo: every check below that
# walks crates walks both.
crate_dirs=()
for c in crates/*/ tools/*/; do
  if [ -d "$c" ]; then crate_dirs+=("$c"); fi
done

# 1. Lines of Rust per crate and in total (tests count: they ship in the repo).
for c in "${crate_dirs[@]}"; do
  n=$(find "$c" -name '*.rs' -print0 | xargs -0 cat | wc -l)
  printf '%-26s %6d LOC (cap %d)\n' "$c" "$n" "$CRATE_CAP"
  if [ "$n" -gt "$CRATE_CAP" ]; then
    echo "FAIL: $c exceeds the per-crate cap"
    fail=1
  fi
done
total=$(find "${crate_dirs[@]}" -name '*.rs' -print0 | xargs -0 cat | wc -l)
printf '%-26s %6d LOC (cap %d)\n' "total" "$total" "$REPO_CAP"
if [ "$total" -gt "$REPO_CAP" ]; then
  echo "FAIL: repo exceeds the total cap"
  fail=1
fi

# 2. CLAUDE.md stays a map, not a novel.
if [ -f CLAUDE.md ]; then
  chars=$(wc -m < CLAUDE.md)
  printf '%-26s %6d chars (cap %d)\n' "CLAUDE.md" "$chars" "$CLAUDE_CAP"
  if [ "$chars" -gt "$CLAUDE_CAP" ]; then
    echo "FAIL: CLAUDE.md exceeds its cap"
    fail=1
  fi
fi

# 3. Zero external dependencies: every dependency is a workspace member,
#    except the wasm-bindgen family in web crates. Read from cargo's own view
#    of the manifests (not their text), so table-form, renamed, target-only,
#    workspace-inherited, git and out-of-tree path deps are all seen; then
#    from Cargo.lock, the resolved graph.
if ! meta=$(cargo metadata --format-version 1 --no-deps --offline); then
  echo "FAIL: cargo metadata could not read the workspace"
  fail=1
else
  # One line per package ({"name":..,"version":..) and per declared dependency
  # ({"name":..,"source":..), in order; cargo writes those two keys first. If
  # the dependency count disagrees with cargo's, fail rather than pass blind.
  marks=$(printf '%s\n' "$meta" | grep -oE '\{"name":"[^"]*","(version|source)":("[^"]*"|null)')
  members=$(printf '%s\n' "$marks" | awk -F'"' '$6 == "version" { printf "%s@%s ", $4, $8 }')
  if [ "$(printf '%s\n' "$meta" | grep -o '"req":"' | wc -l)" -ne "$(printf '%s\n' "$marks" | grep -c '"source":')" ]; then
    echo "FAIL: could not read every dependency from cargo metadata"
    fail=1
  fi
  for c in "${crate_dirs[@]}"; do
    toml="${c}Cargo.toml"
    [ -f "$toml" ] || continue
    name=$(awk -F'"' '/^name = "/ { print $2; exit }' "$toml")
    case " $members" in
      *" $name@"*) ;;
      *) echo "FAIL: $toml is not a workspace member, so cargo never checks it"; fail=1 ;;
    esac
  done
  declared=$(printf '%s\n' "$marks" | awk -F'"' -v members="$members" -v web="$WEB_CRATES" -v webdeps="$WEB_DEPS" '
    BEGIN {
      n = split(members, a, " "); for (i = 1; i <= n; i++) { sub(/@.*/, "", a[i]); member[a[i]] = 1 }
      n = split(web, a, " "); for (i = 1; i <= n; i++) webc["compusophy-" a[i]] = 1
      n = split(webdeps, a, " "); for (i = 1; i <= n; i++) webd[a[i]] = 1
    }
    $6 == "version" { pkg = $4; next }
    $6 == "source" {
      if ($7 == ":null" && ($4 in member)) next
      if ((pkg in webc) && ($4 in webd)) next
      printf "FAIL: %s depends on external crate \047%s\047 (%s)\n", pkg, $4, ($7 == ":null" ? "a path outside the workspace" : $8)
    }')
  if [ -n "$declared" ]; then
    echo "$declared"
    fail=1
  fi
  if [ ! -f Cargo.lock ]; then
    echo "FAIL: no Cargo.lock to check the resolved graph against"
    fail=1
  else
    resolved=$(awk -F'"' -v members="$members" -v allow="$LOCK_ALLOW" '
      BEGIN {
        n = split(members, a, " "); for (i = 1; i <= n; i++) member[a[i]] = 1
        n = split(allow, a, " "); for (i = 1; i <= n; i++) ok[a[i]] = 1
      }
      function check() {
        if (name != "" && src != "" && !(name in ok))
          printf "FAIL: Cargo.lock resolves %s %s from %s\n", name, ver, src
        else if (name != "" && src == "" && !((name "@" ver) in member))
          printf "FAIL: Cargo.lock has %s %s, which is not a workspace member\n", name, ver
        name = ""; ver = ""; src = ""
      }
      /^\[\[package\]\]/ { check(); next }
      /^name = "/ { name = $2 }
      /^version = "/ { ver = $2 }
      /^source = "/ { src = $2 }
      END { check() }
    ' Cargo.lock)
    if [ -n "$resolved" ]; then
      echo "$resolved"
      fail=1
    fi
  fi
fi

# 4. Determinism: in deterministic crates, no floats (types, literals, and
#    *_f32/*_f64 APIs), no hash-ordered or randomly seeded collections, no
#    clocks. Scanned: every code line, plus doc-comment lines inside Rust code
#    fences (doctests compile and run as tests). Plain comments and non-Rust
#    fences (```text) are skipped so docs can explain the rule; strings and
#    trailing comments are not, so write a number like "1.5" another way.
for c in $DETERMINISTIC_CRATES; do
  [ -d "crates/$c" ] || continue
  hits=$(find "crates/$c" -name '*.rs' -print0 | sort -z | xargs -0 awk -v tokens="$DET_TOKENS" '
    function rusty(info,   n, i, w) {
      n = split(info, w, /[ \t,]+/)
      for (i = 1; i <= n; i++)
        if (w[i] != "" && w[i] !~ /^(rust|ignore(-.*)?|should_panic|no_run|compile_fail|test_harness|standalone_crate|edition[0-9]+)$/)
          return 0
      return 1
    }
    FNR == 1 { fence = 0 }
    {
      line = $0; sub(/\r$/, "", line)
      if (line ~ /^[ \t]*\/\/(\/[^\/]|\/$|!)/) {
        body = line; sub(/^[ \t]*\/\/[\/!][ \t]*/, "", body)
        if (body ~ /^(```|~~~)/) {
          if (fence) fence = 0
          else { info = body; sub(/^(```|~~~)[ \t]*/, "", info); fence = 1; rust = rusty(info) }
          next
        }
        if (!fence || !rust) next
        text = body
      } else {
        fence = 0
        if (line ~ /^[ \t]*\/\//) next
        text = line
      }
      t = " " text " "
      if (t ~ ("[^A-Za-z0-9_](" tokens ")[^A-Za-z0-9_]") ||
          t ~ /[^A-Za-z]f(32|64)[^A-Za-z0-9_]/ ||
          t ~ /[^.A-Za-z0-9_][0-9][0-9_]*(\.[0-9]|\.[^.A-Za-z0-9_]|[eE][+-]?[0-9])/)
        print FILENAME ":" FNR ": " line
    }')
  if [ -n "$hits" ]; then
    echo "FAIL: nondeterministic token in crates/$c:"
    echo "$hits"
    fail=1
  fi
done

# 5. Every crate carries the license text: cargo packages only the crate dir,
#    and Apache-2.0 wants the text with every copy.
for c in "${crate_dirs[@]}"; do
  if ! cmp -s LICENSE "${c}LICENSE"; then
    echo "FAIL: ${c}LICENSE is missing or differs from the root LICENSE"
    fail=1
  fi
done

# 6. Privacy: no absolute home-directory paths (they leak the machine's
#    account name) and no email addresses in any file git would commit.
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  leaks=$(git ls-files --cached --others --exclude-standard -z \
    | xargs -0 grep -n -I -E '[A-Za-z]:[/\\]+Users[/\\]|/home/[A-Za-z]|/Users/[A-Za-z]|[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+\.[A-Za-z0-9.-]*[A-Za-z]{2,}' 2>/dev/null)
  if [ -n "$leaks" ]; then
    echo "FAIL: local path or email address in a committable file:"
    echo "$leaks" | cut -c1-160
    fail=1
  fi
fi

if [ "$fail" = 0 ]; then echo "caps: ok"; fi
exit "$fail"
