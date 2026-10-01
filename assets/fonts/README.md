# Fonts

All fonts here are licensed under the SIL Open Font License 1.1 (texts beside
them). Neither declares a Reserved Font Name. The build copies the license
texts into `dist/licenses/`, and the About app credits both.

| file | source | processing |
|---|---|---|
| `Inter-Regular.ttf` | google/fonts `ofl/inter/Inter[opsz,wght].ttf` | instanced at wght 400, opsz 14 |
| `deferred/Inter-SemiBold.ttf` | same | instanced at wght 600, opsz 14 |
| `deferred/JetBrainsMono-Regular.ttf` | google/fonts `ofl/jetbrainsmono/JetBrainsMono[wght].ttf` | instanced at wght 400; wider subset (below) |
| `lazy/symbols-a.ttf` | google/fonts `ofl/notosanssymbols2/NotoSansSymbols2-Regular.ttf` | subset: U+2190-21FF, U+2300-23FF, U+25A0-25FF, U+2600-2613, U+26A0-26A1, U+26AA-26AB, U+2700-27BF, U+2800-28FF, U+2B1D, U+2B50 |
| `lazy/symbols-b.ttf` | google/fonts `ofl/notosanssymbols/NotoSansSymbols[wght].ttf` | instanced at wght 400; subset U+2300-23FF |

The two Inter files are subset with fontTools `pyftsubset` to printable ASCII plus
U+00A0 U+00A9 U+00B7 U+00D7 U+2013 U+2014 U+2018 U+2019 U+201C U+201D
U+2022 U+2026 U+2190-2193 U+2318 U+2325 U+21E7 U+23CE U+2713 U+2715, with
no layout features, no hinting and no glyph names; only the seven tables the
`font` crate reads remain (head, hhea, maxp, cmap format 4, hmtx, loca,
glyf). The copyright notices and licenses ship as the OFL text files instead
(`dist/licenses/` on the site), which the OFL allows.

JetBrains Mono is subset wider, because terminal programs such as Claude CLI
draw with it: U+0020-007E, U+00A0-00FF, U+2000-206F, U+2190-21FF,
U+2300-23FF, U+2500-259F (box drawing and blocks), U+25A0-25FF, U+2700-27BF,
U+E0A0-E0D4 (powerline). The two Noto Symbols files are fallbacks for glyphs
JetBrains Mono lacks (spinner stars, record and check marks, braille). They
live in `lazy/`: the page fetches them when a terminal first opens.

Three load groups, each with its own budget (`scripts/budget.sh`):

- **boot**, inside the wasm: `Inter-Regular.ttf` (about 8 KB gzipped). The
  first frame needs only this.
- **deferred**, fetched right after the first frame: `deferred/` (Inter
  SemiBold for titles and headings, JetBrains Mono for terminals and code;
  about 24 KB). Until they arrive, bold text draws in Regular and monospace
  cells stay empty.
- **lazy**, fetched on demand: `lazy/` (the symbol fallbacks; about 39 KB).

To regenerate: `python -m fontTools.varLib.instancer <var.ttf> wght=400
opsz=14 -o full.ttf`, then `pyftsubset full.ttf --unicodes=<list above>
--layout-features='' --no-hinting --notdef-outline --no-glyph-names
--drop-tables+=GPOS,GSUB,GDEF,STAT,DSIG,gasp,prep,fpgm,cvt,hdmx,VDMX,LTSH,kern,meta,FFTM,vhea,vmtx,name,OS/2,post`.
