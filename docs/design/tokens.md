# Tokens

Defined as variables in `phpm.pen`. A web build should carry the same names
as CSS custom properties, so a value changed in one place is changed in the
other.

Two modes, both validated. Light is primary (docs read better on paper-white
for long technical pages); dark is a real second mode, not an inverted
afterthought, because terminal-first developer tools live in dark terminals.

## Colour

| Token | Light | Dark | Use |
|---|---|---|---|
| `bg` | `#FAF9F7` | `#121113` | Page background. Warm off-white / near-black, never pure white or pure black |
| `surface` | `#FFFFFF` | `#19181B` | Cards, panels, the benchmark table |
| `surface-2` | `#F1EFEA` | `#201F23` | Code block fill, table heads, quiet buttons |
| `line` | `#E4E1D9` | `#2C2A30` | Hairlines, table and card borders |
| `line-2` | `#D2CEC3` | `#3A3840` | Outline-button borders |
| `ink` | `#17151B` | `#F2F0EC` | Primary text |
| `ink-2` | `#4B4754` | `#A8A4AC` | Secondary text, body copy |
| `ink-3` | `#716C7D` | `#716D78` | Labels, table heads, captions. 3.7-4.2:1 on `bg` — large or semibold text only, never small body copy |
| `accent` | `#A2650F` | `#F2A93C` | The one brand colour: primary button, links, active nav, the amber in every benchmark bar and number |
| `accent-ink` | `#FFFFFF` | `#1A1408` | Text/icon colour on top of a solid `accent` fill |
| `accent-soft` | `#A2650F1F` | `#F2A93C26` | Highlighted table row (phpm's own row), callout fill |

Status colours are fixed across both modes (not themed) and never reused for
anything else: `good` `#0CA30C`, `warning` `#C98500`, `critical` `#D03B3B`,
`skip` `#8A8578` / `#8B8690`. They pair with an icon or a word
("identical", "4 files differ"), never colour alone — a dot plus a label in
every table cell. This follows the project's own dataviz convention: status
is state, not identity, and is never cycled as a categorical hue.

**Why amber.** Decision 0001 picked the name because it "reads close to
pnpm" — the same global-store-and-clone mental model. pnpm's own brand leans
into a warm orange/amber. Carrying that into phpm's colour is not
decorative: it is the second thing (after the name) that tells a PHP
developer who already uses pnpm what kind of tool this is before they read a
word. Every other colour in the system is either ink (text) or a reserved
status colour. No blue, no purple, no gradient — one brand colour, used with
conviction rather than diluted across decoration.

Both accent steps were checked against WCAG contrast: light `#A2650F` on
`bg` is 4.53:1 and on white button text 4.77:1 (passes AA body text); dark
`#F2A93C` on `bg` is 9.43:1 and on `accent-ink` 9.89:1. The categorical
"good / warning / critical" set is the project's dataviz skill's validated
default status palette, carried through unchanged because phpm's own
decision 0006 already treats published numbers as a trust mechanism —
inventing a second status palette would just be more surface area to get
wrong.

## Type

| Token | Family | Use |
|---|---|---|
| `font-display` | Space Grotesk | Headlines, the wordmark, big numbers ≥ 24px |
| `font-body` | IBM Plex Sans | Prose, labels, nav, button text |
| `font-mono` | IBM Plex Mono | Code, CLI output, benchmark numbers, table figures, version strings |

All three are on Google Fonts, no paid licence, no variable-font tooling
needed for a static docs build. Space Grotesk reads geometric and slightly
technical without being a display gimmick — it is legible at both 80px
(poster) and 17px (nav wordmark). IBM Plex Sans and Plex Mono share the same
underlying design, so prose and the numbers sitting next to it never feel
like two unrelated typefaces glued together — relevant on a site where body
text and benchmark tables sit in the same paragraph constantly.

Scale (px): 12 (micro labels, table heads, uppercase) · 13 (small UI, mono
figures) · 14 (body, button text) · 15-17 (lead paragraph) · 22-26 (H4,
kicker) · 34 (H2, section titles) · 46-52 (H1, screen headline) · 80 (poster
headline only). `letterSpacing` goes slightly negative (−0.5 to −2) only at
the two largest display sizes; everything else is default tracking. Table
heads and kickers get +0.2 tracking with uppercase.

## Space and shape

Space: 4 · 8 · 12 · 16 · 24 · 32 · 48 · 64 (`sp-1` through `sp-8`). Page
padding on a 1440 screen is 56-64px top/sides; section gap is 28-36px;
component-internal gap is 8-20px depending on density.

Radius: 6px (`radius-sm` — buttons, inputs, badges, mini code blocks), 10px
(`radius-md` — cards, the benchmark table, the code block, callouts), 16px
(`radius-lg` — large hero-level containers, used sparingly). No pill
buttons: a Rust CLI's audience reads rounded rectangles as credible and
pills as consumer-SaaS. One radius scale, applied consistently — never a
pill button next to a square card on the same screen.

## Components

**Buttons.** Flat solid fill only, no gradient, no glow, no bevel — a hard
rule from the brief and consistent with every reference site surveyed
(Astral, Biome, Bun, Zed). Four variants: `Button/Primary` (solid `accent`
fill, `accent-ink` text — one per section, the loudest thing on the
screen), `Button/Secondary` (solid `surface-2` fill, `ink` text),
`Button/Outline` (`line-2` 1px border, transparent fill, `ink` text — the
GitHub / docs / "read more" action), `Button/Ghost` (no fill or border,
`ink-2` text — cancel, inline actions). Icon slot is optional and disabled
by default.

**Badge.** A 6px status dot plus a mono label in a `surface-2` pill-less
chip (`radius-sm`). Used for sweep/identity results ("identical", "4 files
differ") — dot colour carries the status palette, the word always ships
with it.

**Code block.** `surface-2` fill, `radius-md`, a header bar with three
traffic-light dots and a mono filename/label, body in `font-mono` 12-13px.
Command lines get an `ink-3` `$` prompt and `accent` command text; plain
output is `ink-2`; a result line that matters (byte-identical, a pass) is
set in `good`. This is the single most load-bearing component in the whole
system — phpm has no release yet, so "copy this into your terminal" is the
actual product experience today, not a decorative screenshot.

**Benchmark table.** `surface` fill, `radius-md`, a `surface-2` head row
with uppercase mono-adjacent labels, data rows with a hairline bottom
border. phpm's own row is the only one with an `accent-soft` background and
`accent` numbers in bold — the eye should land there without needing a
"winner" badge. Numbers are right-aligned-reading (fixed-width mono columns)
so the table scans as a column of digits, the way a terminal does.

**Comparison-chart treatment.** Horizontal bars, not a bar chart library
look: thin track in `surface-2`, flat fill, rounded data-end only
(`radius-sm`-equivalent corner radius on the fill, matching the dataviz
skill's "rounded data-ends anchored to the baseline" mark spec), a 2px gap
never crossed by colour. phpm's bar is `accent`; every competitor's bar is
flat `ink-3` grey. This is a deliberate departure from a rainbow categorical
palette: the story is "one tool is different," not "five tools of equal
visual weight," so colour is spent on identity (phpm) rather than cycled
across every series. Mono value label at the end of each bar.

**Badges / status dots for the comparison matrix.** Plain mono text cells
("1.9x", "no — 4 files differ"), not icon grids — a feature-comparison
table with real numbers reads as more honest than a wall of green
checkmarks, and matches decision 0006 and 0004's own voice: specific,
qualified claims, not a marketing checklist.

**Nav.** 64px bar, `bg` fill, 1px `line` bottom border. Mark + wordmark
left, four text links centre-right, one `Button/Outline` ("GitHub") far
right. No mega-menu, no dropdown — the whole docs site is five sections
deep at this stage of the project, and pretending otherwise would be
dishonest about where phpm actually is.

**Recurring system detail — the ring motif.** The two-arc logo mark repeats
at 14-16px as a small bullet in place of an uppercase "eyebrow" label
(kicker rows: a ring-dot plus one word, e.g. "benchmarks", "quickstart"),
and as a divider in the type/colour specimen rows. This is the one
decorative motif the system allows itself, and it is the same mark as the
logo — not a second invented device — so it reads as the brand's own
handwriting rather than generic developer-tool iconography (a chevron, a
dot, a hash).

## What this is not

No dark-mode-with-a-blue-accent default. No AI-purple glow. No pill
buttons. No three equal feature cards. No decorative gradients anywhere —
every fill in every component is a flat colour, which also means it costs
nothing to hand a design like this to a static-HTML or mdBook build later:
there is no gradient mesh or blur effect that a plain CSS build would have
to approximate.
