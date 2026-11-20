# Dashboard design system

Status: proposed for issue #35. Applies to every page served by `dashboard`.

## Direction

The dashboard is a tool the owner glances at, on a phone or a desktop, a few
times a day. It uses a restrained palette: quiet neutral surfaces, one teal
accent for navigation, links, focus, and selection, and two status colours
(alert, warning) kept rare so they stand out. The type is the platform's
system face, with tabular numerals for readings. The surfaces are flat, with
1 px borders and no shadows, gradients, motion, or icons. Status is always
spelled out in words, and colour only reinforces the words.

## Tokens

All tokens are CSS custom properties on `:root`. Dark values replace light
values inside `@media (prefers-color-scheme: dark)`. Declare
`color-scheme: light dark` on `:root` and add `<meta name="color-scheme"
content="light dark">`, so native controls, scrollbars, and form widgets
follow the theme.

### Colour

| Token | Role | Light | Dark |
|---|---|---|---|
| `--bg` | page canvas | `#f5f6f4` | `#111413` |
| `--surface` | cards, nav bar, table | `#ffffff` | `#1a1e1c` |
| `--sunken` | notices, table header, unselected segment | `#eceeeb` | `#232826` |
| `--text` | primary text, values | `#1a1d1b` | `#e6e9e7` |
| `--text-muted` | labels, units, timestamps, axes | `#565d59` | `#a2aaa5` |
| `--border` | decorative separators, card edges | `#d5d9d5` | `#323936` |
| `--control` | boundaries of inputs, chips, segments | `#767d78` | `#78817c` |
| `--accent` | links, current nav, focus ring, primary button | `#0b6b5d` | `#5fc9b3` |
| `--on-accent` | text on `--accent` | `#ffffff` | `#0b1f1b` |
| `--accent-bg` | current nav item, checked segment, `::selection` | `#e1f0ec` | `#173430` |
| `--alert` | active-alert badge text and border | `#a5231b` | `#ff9e92` |
| `--alert-bg` | active-alert badge fill | `#fbe8e5` | `#3d1915` |
| `--warn` | warning badge and notice text and border | `#7a4f00` | `#f0c063` |
| `--warn-bg` | warning badge and notice fill | `#fbf0d6` | `#352a10` |
| `--chart-1` … `--chart-8` | room lines on History | see below | see below |

Chart series colours, from `--chart-1` to `--chart-8`:

- Light: `#1f6fb4 #c2402a #2c8a3e #8a5cc2 #b06a00 #0f8a8a #c23a8a #5d6a20`
- Dark: `#6aaef0 #f08a72 #6cc47c #b59af0 #e8a64a #4fc8c8 #f08ac4 #b5c45a`

The Outside line uses `--text-muted` and is dashed.

### Contrast (WCAG 2.1, computed)

| Pair | Use | Light | Dark | Needs |
|---|---|---|---|---|
| `--text` on `--bg` | body | 15.68 | 15.15 | 4.5 |
| `--text` on `--surface` | card text, values | 17.00 | 13.78 | 4.5 |
| `--text` on `--sunken` | notice text | 14.57 | 12.24 | 4.5 |
| `--text-muted` on `--bg` | page meta | 6.24 | 7.79 | 4.5 |
| `--text-muted` on `--surface` | units, labels | 6.76 | 7.08 | 4.5 |
| `--text-muted` on `--sunken` | table header | 5.79 | 6.29 | 4.5 |
| `--accent` on `--bg` | links, focus ring | 5.91 | 9.26 | 4.5 |
| `--accent` on `--surface` | links, nav text, focus ring | 6.41 | 8.42 | 4.5 |
| `--accent` on `--sunken` | focus ring on sunken | 5.49 | 7.48 | 3 |
| `--on-accent` on `--accent` | primary button | 6.41 | 8.56 | 4.5 |
| `--accent` on `--accent-bg` | current nav, checked segment | 5.46 | 6.68 | 4.5 |
| `--text` on `--accent-bg` | selected text | 14.47 | 10.94 | 4.5 |
| `--alert` on `--alert-bg` | alert badge | 6.22 | 7.83 | 4.5 |
| `--alert` on `--surface` | alert badge border | 7.35 | 8.47 | 3 |
| `--warn` on `--warn-bg` | warning badge, notice | 6.29 | 8.34 | 4.5 |
| `--text` on `--warn-bg` | notice body | 15.00 | 11.53 | 4.5 |
| `--warn` on `--surface` | warning badge border | 7.13 | 9.97 | 3 |
| `--control` on `--surface` | input and chip borders | 4.22 | 4.20 | 3 |
| `--control` on `--bg` | input borders on canvas | 3.89 | 4.61 | 3 |
| `--control` on `--sunken` | unselected segment border | 3.62 | 3.73 | 3 |
| `--chart-1…8` on `--surface` | chart lines (lowest of 8) | 4.18 | 6.89 | 3 |
| `--text-muted` on `--surface` | Outside line, axes | 6.76 | 7.08 | 3 |

`--border` is decorative. A card is identified by its contents and its
`--surface` fill, so the border does not need 3:1. Never use `--border` as the
only boundary of a control.

### Spacing (4 px base)

| Token | Value | Use |
|---|---|---|
| `--space-1` | 0.25rem | value to unit, badge padding (block) |
| `--space-2` | 0.5rem | inside a card between lines, badge padding (inline) |
| `--space-3` | 0.75rem | card padding, card grid gap, list row padding |
| `--space-4` | 1rem | page side padding (phone), gap between page header and first section |
| `--space-5` | 1.5rem | page side padding (desktop), gap between sections |
| `--space-6` | 2rem | space above a visible section heading |

### Type

The font stack is `--font-sans: system-ui, -apple-system, "Segoe UI", Roboto,
"Noto Sans", sans-serif` and `--font-mono: ui-monospace, "SF Mono", Menlo,
Consolas, monospace`. The mono stack is used only for MAC addresses. The
dashboard does not use web fonts and does not vendor a font, because the
system stack costs no bytes and reads well on every target device.

| Token | Size | Line height | Weight | Use |
|---|---|---|---|---|
| `--text-sm` | 0.875rem | 1.4 | 400 / 600 | labels, units, timestamps, badges, table cells, nav (phone) |
| `--text-base` | 1rem | 1.5 | 400 | body, notices, form labels, nav (desktop) |
| `--text-lg` | 1.25rem | 1.2 | 600 | page title (`h1`), visible section headings (`h2`) |
| `--text-xl` | 1.75rem | 1 | 600 | room temperature |
| `--text-2xl` | 2.25rem | 1 | 600 | outside temperature |

Every reading, time, and table number uses `font-variant-numeric:
tabular-nums`. The page does not use uppercase labels, letter-spacing tricks,
or eyebrows.

### Radii, layout, breakpoint

| Token | Value | Use |
|---|---|---|
| `--radius-sm` | 0.375rem | badges, inputs, segments, chips, buttons |
| `--radius-md` | 0.75rem | cards, notices, table wrapper |
| `--page-max` | 72rem | content max width, centred |
| `--tap` | 2.75rem | minimum height of every interactive target (44 px) |

There is one breakpoint, at `48rem` (768 px). Below it is the phone layout and
at or above it is the desktop layout. Custom properties cannot be used in media
queries, so write `@media (min-width: 48rem)` literally. Tablets use the
desktop layout. Grids are fluid on both sides of the breakpoint.

The token values live in `crates/dashboard/assets/app.css`.

## Components

### App shell and nav

- The DOM order is: skip link, `<header>` containing `<nav aria-label="Main">`,
  `<main id="main">`. The shell comes from one base template, so every page
  has the same shell.
- The nav has five links, always in this order: Overview `/`, History
  `/history`, Alerts `/alerts`, Health `/health`, Imports `/imports`. Every
  page shows all five, including the current page.
- The header is `position: sticky; top: 0`, with `--surface` fill and a
  1 px `--border` bottom edge. The nav therefore stays one tap away on long
  pages, such as Health.
- Phone: the bar is 48 px tall. The five links share the width equally
  (`grid-template-columns: repeat(5, minmax(0, 1fr))`, so the nav stays one
  row down to 320 px). Links use `--text-sm` 600 and are centred; below 360 px
  the font steps down to 0.8125rem so "Overview" fits in a 64 px column. No brand text is shown.
- Desktop: "Home monitor" appears as plain text (`--text-base` 600, not a
  link) at the left, and the links follow inline, each at least 44 px tall
  with `--space-4` inline padding. The bar content is capped at `--page-max`.
- A link is `--text-muted`. On hover it is `--text`.
- The current page has `aria-current="page"`, `--accent` text, `--accent-bg`
  fill, weight 600, and a 3 px `--accent` bar on its bottom edge. The bar and
  the weight carry the state without colour.
- The skip link "Skip to content" points to `#main`. It is visually hidden
  until focused, then shown at the top left above the bar, with `--accent`
  fill and `--on-accent` text.

### Page header

- An `h1` (`--text-lg`) on the left. It always matches the nav label:
  Overview, History, Alerts, Health, Imports.
- Optional meta on the same row, right-aligned, in `--text-sm` and
  `--text-muted`: "Updated 13:42", the render time in Europe/Paris. Overview
  Alerts and Imports show it because they auto-refresh.
- `<title>` is "<Page> · Home monitor".

### Card

- `--surface` fill, 1 px `--border`, `--radius-md`, `--space-3` padding.
  Cards are never nested and do not have shadows.
- Cards sit only in a `card-grid`: `display: grid; gap: var(--space-3);
  grid-template-columns: repeat(auto-fill, minmax(min(10rem, 100%), 1fr))`.
  At or above 48rem the minimum is `12rem`. A card can take the modifier
  `card--wide` (`grid-column: span 2`, or `1 / -1` when the grid has fewer
  than 3 columns). The grid can take the modifier `card-grid--jobs`, whose
  minimum is `minmax(min(20rem, 100%), 1fr)` on both sides of the breakpoint.
- The card title is an `h3` in `--text-sm` 600 `--text`, with a badge at its
  right when one applies.
- Use a card for one subject's current state (a room, outside, an unknown
  sensor, and later a #36 card). Do not use a card for lists, forms, or
  notices.

### Metric value with unit

- The number uses `--text-xl` (room) or `--text-2xl` (outside), weight 600,
  tabular. The unit follows after `--space-1`, in `--text-sm` and
  `--text-muted`, top-aligned. Example: **21.3** °C.
- A secondary metric (humidity) is one line in `--text-sm` `--text-muted`:
  "51.5 % humidity".
- A missing value renders as "—" with a visually hidden "no reading", so a
  screen reader does not read "dash".
- Formats do not change: one decimal for °C and %.

### Status badge

An inline label: `--text-sm` 600, `--radius-sm`, padding `--space-1`
`--space-2`, and a 1 px border in the text colour. The word always carries
the meaning, and the dashboard has no icon-only status.

| Variant | Colours | Text | Where |
|---|---|---|---|
| `badge--alert` | `--alert` on `--alert-bg` | Too hot · Too cold · Offline · Login expired · Failed | active alerts and Garmin jobs that gave up |
| `badge--warn` | `--warn` on `--warn-bg` | Not reporting | room, outside, and sensor cards whose `Freshness` is `NotReporting` |
| `badge--neutral` | `--text-muted` on `--sunken` | Too hot · Too cold · Offline · Login expired | resolved alerts |

The code has only two freshness states (`Reporting` and `NotReporting`), so
"stale" and "not reporting" are the same state and use one badge. A card that
is not reporting shows its last values in `--text-muted` when the values
exist, and "—" when they do not.

### Alert row

Active and resolved alerts use the same component: a `<ul>` with one `<li>`
per alert. The list sits on `--surface`, with 1 px `--border` around it,
`--radius-md`, and 1 px `--border` between rows. Each row has `--space-3`
padding and a minimum height of `--tap`.

- Row content, in reading order: badge, subject (`--text-base` 600), detail
  (`--text-sm` `--text-muted`), time (`--text-sm` `--text-muted`, tabular).
  A subject is never truncated; it wraps.
- Active detail: "21.3 °C at last notification" when known. Active time:
  "since 2026-01-15 13:30".
- Resolved time: "2026-07-15 10:00 – 2026-07-16 01:30".
- Phone: badge and subject on line 1 (a long subject wraps onto more lines),
  then the detail, then the time on its own line, left-aligned. On the
  overview the detail is hidden, so each row has two lines.
- Desktop (48rem and up): one line, with the time pushed right. A row wraps
  rather than truncates when it does not fit.

### Data table (Health)

- A `<table>` with `<caption>` (visually hidden is allowed) and `<th
  scope="col">` headers in `--text-sm` 600 `--text-muted` on `--sunken`.
  Cells are `--text-sm`, padded `--space-2` `--space-3`, with a 1 px
  `--border` row separator and no zebra stripes.
- Numbers are right-aligned and tabular. Dates and text are left-aligned.
- The table sits in a wrapper (`--surface`, `--radius-md`, 1 px `--border`)
  with `overflow-x: auto`, `tabindex="0"`, `role="region"`, and
  `aria-labelledby` pointing to the caption, so a keyboard user can scroll it.
  The first column is `position: sticky; left: 0` on `--surface`.
- Use the table only for a matrix of values that are compared down columns.
  A list of events uses the alert row.

### Form controls (History filter)

- Each group is a `<fieldset>` with a visible `<legend>` (`--text-sm` 600).
- Segmented choice (Metric, Period): native radios inside labels. The input
  is visually hidden but still focusable, and the label's `<span>` draws the
  segment. Segments join in one row: `--sunken` fill, 1 px `--control`
  border, `--radius-sm` on the outer ends, height ≥ `--tap`. The checked
  segment has `--accent-bg` fill, `--accent` text, weight 600, and a 2 px
  `--accent` border. Arrow keys move between options natively.
- Checkbox chip (rooms, Outside): a label with a visible native checkbox
  (`accent-color: var(--accent)`) and its text, 1 px `--control` border,
  `--radius-sm`, height ≥ `--tap`, wrapping in a flex row with `--space-2`
  gap.
- Primary button: `--accent` fill, `--on-accent` text, `--text-base` 600,
  `--radius-sm`, height ≥ `--tap`, `--space-4` inline padding. It is full
  width on phone and auto width on desktop. Use it once per page at most.
  Only History has one.

### Notice (empty and unavailable states)

- A block in `--radius-md`, `--space-3` padding, `--text-base`. It has a bold
  first sentence that names the state, followed by at most one sentence about
  what happens next.
- Neutral (empty): `--sunken` fill, `--text`. Examples: "No active alerts.",
  "No readings in this period."
- Warning (unavailable): `--warn-bg` fill, 1 px `--warn` border, `--text`
  body, and a leading `badge--warn` "Unavailable". Use it when a dependency
  (store or alerter) did not respond. It never stands in for "no data".
- Notices replace only the region whose data is missing. The rest of the page
  still renders.

### Focus ring

- `:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }`
  applies to every interactive element. Inside the nav bar and on segments,
  use `outline-offset: -2px` so the ring is not clipped. Never remove the
  outline without this replacement.

### Browser surfaces

- `::selection` uses `--accent-bg` with `--text`. `caret-color` is
  `--accent`.
- Links are `--accent`, underlined, with `text-underline-offset: 0.2em`. Nav
  links are not underlined, because the current-page bar replaces the
  underline.
- `scrollbar-color: var(--control) transparent`.
- Body: `--bg`, `--text`, `--font-sans`, `--text-base`.
- Respect `prefers-reduced-motion`. No motion is defined, so nothing needs to
  be removed.

## Rules

1. Use only tokens. A raw hex value, raw px value, or extra font size in page
   CSS is a defect. The exceptions are the 1 px border width, the 3 px nav
   bar and the nav font below 360 px.
2. Each view has one visual priority. Accent fill appears only on the current
   nav item, the checked segment, and the one primary button.
3. Status is words first. Every coloured state has text, and a badge without
   text is not allowed.
4. Use `--alert` only for active alerts and failed Garmin jobs, and `--warn`
   only for "not reporting" and "unavailable".
5. "Unavailable" (the dependency failed) and "empty" (no data) are different
   states and always use different copy.
6. A new overview card joins `card-grid` as a `card` and does not add a new
   container.
