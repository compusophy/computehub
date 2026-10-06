"""What Studio's coder reads of a reply, mirrored to the character from programs/coder
(ai::blocks, edits::{marked, honest, program}) and applang's lexer, so generate.py stops a
sample where Studio stops reading and cuts its reply where the grade stays the same. No torch:
test_blocks.py runs it anywhere."""

# What Rust's char::is_whitespace, and so str::trim, takes: Unicode White_Space. Python's
# str.strip() also takes \x1c-\x1f, so it would read some lines as Rust does not.
WS = "\t\n\x0b\x0c\r \x85\xa0 " + "".join(chr(c) for c in range(0x2000, 0x200b)) \
    + "    　"
MARK = "<<<<<<<"


def lines(text):
    """str::split_inclusive('\\n'): each line with its newline, the last without one; never
    split on \\r, \\x0b, \\x85, U+2028 or the other breaks str.splitlines() takes."""
    at = 0
    while at < len(text):
        nl = text.find("\n", at)
        end = len(text) if nl < 0 else nl + 1
        yield text[at:end]
        at = end


def blocks(reply):
    """coder::ai::blocks: each fenced block of reply whose info string is app, in order, as
    (text, closed, end). An opener is a line that, trimmed, is ``` then (trimmed) app; while one
    is open, any line that, trimmed, begins with ``` closes it. end is just past the closing
    fence line, before its line break (the reply's end for an open block, only ever the last)."""
    out, at, start = [], 0, None
    for line in lines(reply):
        t = line.strip(WS)
        fence = t[3:] if t.startswith("```") else None
        if start is None:
            if fence is not None and fence.strip(WS) == "app":
                start = at + len(line)
        elif fence is not None:
            out.append((reply[start:at], True, at + len(line.rstrip("\r\n"))))
            start = None
        at += len(line)
    if start is not None:
        out.append((reply[start:], False, len(reply)))
    return out


def marked(text):
    """coder::edits::marked: a line of text begins, after whitespace, with an edit block's
    <<<<<<<."""
    return any(ln.lstrip(WS).startswith(MARK) for ln in text.split("\n"))


def honest(src):
    """coder::edits::honest: src's first token is a comment. applang's lexer skips only space,
    tab, \\n and \\r before it; // and /* (closed or not) open one."""
    return src.lstrip(" \t\n\r").startswith(("//", "/*"))


def program(reply):
    """coder::edits::program, the program iq grades: (text, closed) of the first closed block
    without edit markers that is honest; else the longest block without markers (in UTF-8
    bytes, the first of equals), to the reply's end if open; else None."""
    long = None
    for text, closed, _ in blocks(reply):
        if marked(text):
            continue
        if closed and honest(text):
            return text, closed
        if long is None or len(text.encode("utf-8")) > len(long[0].encode("utf-8")):
            long = (text, closed)
    return long


def block_end(reply):
    """Where Studio stops reading reply: just past the closing fence of its first closed block
    without edit markers that is honest (Make::program_in, the block edits::program takes), or
    None. reply[:block_end(reply)] holds that block whole, and edits::program takes it there too."""
    for text, closed, end in blocks(reply):
        if closed and not marked(text) and honest(text):
            return end
    return None
