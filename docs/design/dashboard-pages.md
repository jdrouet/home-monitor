# Dashboard pages

Status: proposed for issue #35. The components and tokens are defined in
[design-system.md](design-system.md).

The redesign keeps every route, query parameter, data source, and format. It
replaces the look, adds a shared shell and nav, adds active alerts to the
overview, and gives each failure a rendered state instead of a bare
`store unavailable` text response.

## Shared to all pages

- Shell: the skip link, a sticky header with the nav, then `main`. Every page
  links to the other three in one tap, and the current page is marked with
  `aria-current="page"` plus a visual state (design-system.md, App shell).
- Page side padding is `--space-4` on phone and `--space-5` on desktop. Main
  content is capped at `--page-max` and centred.
- Focus order: skip link, Overview, History, Alerts, Health, then `main` in
  DOM order. The visual order matches the DOM order on both layouts.
- Store unavailable: the page renders the full shell, the page header, and a
  warning notice in the data region. The HTTP status stays 502, as today. A
  page that auto-refreshes keeps its refresh, so it recovers without the
  owner's action.
- Auto refresh periods are unchanged: Overview 60 s, Alerts 60 s, Health
  300 s, History none. An inline script reloads the page only while it is
  visible and no form field has focus (ADR-0026). No page uses a
  `<meta http-equiv="refresh">`.

## Overview `/`

Content order:

1. Page header: "Overview" with "Updated HH:MM".
2. Active alerts.
3. Room cards, in config order.
4. The Outside card.
5. Unknown sensors, only when present.

The first three blocks are the glance zone. Unknown sensors may fall below
the fold.

The overview reads the alerter, as `/alerts` does. The store request and the
alerter request should run concurrently, so a slow alerter (5 s timeout)
does not add to the store's time.

### Phone (390 × 844)

```
┌──────────────────────────────────────┐ 0
│ Overview │ History │ Alerts │ Health │ 48  sticky nav, current = Overview
├──────────────────────────────────────┤
│ Overview                Updated 13:42│ 92  page header
│ ┌──────────────────────────────────┐ │
│ │[Too hot] Bedroom            13:30│ │ 136 alert rows, 44 px each, max 3
│ │[Offline] Office             10:00│ │ 180
│ └──────────────────────────────────┘ │
│ ┌───────────────┐ ┌───────────────┐  │ 196
│ │Bedroom        │ │Kitchen        │  │
│ │21.3 °C        │ │19.8 °C        │  │ room cards, 2 columns, ~96 px
│ │51.5 % humidity│ │48.0 % humidity│  │
│ └───────────────┘ └───────────────┘  │ 292
│ ┌───────────────┐ ┌───────────────┐  │
│ │Office [Not rep│ │Living room    │  │
│ │—              │ │20.4 °C        │  │
│ │—              │ │50.2 % humidity│  │
│ └───────────────┘ └───────────────┘  │ 400
│ ┌───────────────┐ ┌───────────────┐  │
│ │Bathroom       │ │Kids room      │  │
│ │22.0 °C        │ │20.9 °C        │  │
│ └───────────────┘ └───────────────┘  │ 508
│ ┌──────────────────────────────────┐ │
│ │Outside                           │ │ card--wide, ~104 px
│ │4.8 °C          78.0 % humidity   │ │
│ │Overcast · today 2.4–11.0 °C      │ │
│ └──────────────────────────────────┘ │ 624
└──────────────────────────────────────┘ 640 + bottom padding
```

Vertical budget, in CSS px, from the stylesheet and not measured on a device:
nav 48, page top padding 16, header 40 (title 24 and a 16 gap), alerts 48
(the "No alerts" notice) or 229 (three rows of 75), gap 24, rooms (3 rows of
103, with 2 gaps of 12) 332, gap 24, outside 111, bottom padding 16. The
total is about 658 px with no alerts and 839 px with three alert rows and six
rooms. With four rooms it is about 544 px and 725 px.

The target is a visible height of about 690 px. Chrome on Android and Safari
with the compact tab bar give roughly that height at 390 × 844. With six
rooms and three alert rows, the alerts and every room card end at about
689 px, and the Outside card starts at about 713 px, so it needs a short
scroll. Everything just fits in the full 844 px.
A card with no readings shows one "—" line next to its badge, so it is
shorter than its neighbours; its row keeps the height of the taller card.

- Alert rows on the overview take two lines on phone: badge and the full
  subject, then the time below. The time keeps the existing
  `since YYYY-MM-DD HH:MM` format; the sketch shortens it only for width. A
  subject is never truncated; a long one wraps. From 48rem the row is one
  line. The "°C at last notification" detail is hidden below 48rem.
- With more than 3 active alerts, the overview shows the 2 oldest (by onset,
  as on `/alerts`) and a third row "3 more active alerts", which links to
  `/alerts`.
- With an odd number of rooms, the last room row has one empty cell. The
  Outside card always spans the full row.

### Desktop (≥ 48rem, shown at 1280 × 800)

```
┌──────────────────────────────────────────────────────────────────────────┐
│ Home monitor   [Overview]  History   Alerts   Health                     │ sticky
├──────────────────────────────────────────────────────────────────────────┤
│  Overview                                                 Updated 13:42  │
│  ┌────────────────────────────────────────────────────────────────────┐  │
│  │ [Too hot] Bedroom   21.3 °C at last notification  since 2026-…13:30│  │
│  │ [Offline] Office                                  since 2026-…10:00│  │
│  └────────────────────────────────────────────────────────────────────┘  │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐        │
│  │Bedroom   │ │Kitchen   │ │Office [NR│ │Living rm │ │Bathroom  │        │
│  │21.3 °C   │ │19.8 °C   │ │—         │ │20.4 °C   │ │22.0 °C   │        │
│  │51.5 % hum│ │48.0 % hum│ │—         │ │50.2 % hum│ │55.1 % hum│        │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘ └──────────┘        │
│  ┌──────────┐ ┌─────────────────────┐                                    │
│  │Kids room │ │Outside              │                                    │
│  │20.9 °C   │ │4.8 °C   78.0 % hum. │                                    │
│  │49.0 % hum│ │Overcast · 2.4–11.0  │                                    │
│  └──────────┘ └─────────────────────┘                                    │
│                                                                          │
│  Unknown sensors                                                         │
│  ┌─────────────────┐                                                     │
│  │A4:C1:38:00:00:09│                                                     │
│  │18.0 °C          │                                                     │
│  └─────────────────┘                                                     │
└──────────────────────────────────────────────────────────────────────────┘
```

On desktop the grid has 5 or 6 columns at `--page-max` (`minmax(12rem, 1fr)`),
the Outside card spans 2 columns, and active alert rows show their detail.

### Headings

- `h1` "Overview".
- A visually hidden `h2` "Active alerts" on the alerts block.
- A visually hidden `h2` "Rooms and weather" on the grid.
- An `h3` for each card (room name, "Outside", MAC).
- A visible `h2` "Unknown sensors" (`--text-lg`, `--space-6` above).

### States

| State | What the owner sees |
|---|---|
| No active alerts | A neutral notice of one line: "No active alerts." (44 px, so the layout does not jump) |
| Alerter unavailable | A warning notice in the alerts slot: "**Alerts unavailable.** The alerter did not respond; room values below are current." Rooms and weather render normally, with status 200. |
| Store unavailable | A warning notice in place of the grid: "**Room values unavailable.** The store did not respond. This page retries every minute." The alerts block still renders when the alerter answered. Status 502. |
| Store and alerter unavailable | Both notices, in order. Status 502. |
| Room not reporting | The card keeps its grid position, shows a `badge--warn` "Not reporting", and shows its last values in `--text-muted` or "—". |
| Weather not reporting | The Outside card shows `badge--warn` "Not reporting" and the line "Weather unavailable". |
| A value missing while reporting | "—" with a visually hidden "no reading". |
| Unknown sensors | A visible `h2` section after the grid, with one line in `--text-muted`: "Not assigned to a room in config.toml." The cards use the same component, with the MAC in `--font-mono` as the title. The section is hidden when there are none. |
| No rooms configured | The grid holds only the Outside card. |

### Slots kept for later (not rendered now)

- **Cards (#36)**: future cards join `card-grid` after the Outside card, as
  `card` or `card--wide`. They need no new container. The order rule is
  "alerts, then cards", with rooms and outside being the first cards.
- **Messages**: a full-width region between the alerts block and
  `card-grid`, using the notice component. Nothing is rendered and no markup
  or comment is emitted until the messages feature exists. This document is
  the record of the slot.

## History `/history`

Content order: page header "History", filter form, chart card.

- Filter form, unchanged GET to `/history` with the same parameters:
  - "Metric": a segmented radio group, Temperature | Humidity
    (`name="metric"`, same values).
  - "Period": a segmented radio group, 24 h | 7 d | 30 d | 1 y
    (`name="period"`, same values).
  - "Lines": checkbox chips, one per room and then Outside (`name="room"`,
    `name="outside"`).
  - The button "Update chart".
  - Display labels are capitalised words; the submitted values stay the
    enum strings.
- Chart card: an `h2` that states the selection ("Temperature, last 24
  hours"), the uPlot chart, and uPlot's legend below.
- Chart rendering:
  - The chart is 240 px tall on phone and 360 px on desktop. Its width is
    the card's content width.
  - The chart is redrawn on window resize and on a change of
    `prefers-color-scheme`.
  - Series colours, axis and tick text (`--text-muted`), and grid lines
    (`--border`) are read from the CSS custom properties, not hard-coded.
  - The Outside line is dashed `--text-muted`. Room lines cycle through
    `--chart-1…8` and alternate solid and dashed by index, so adjacent lines
    differ by more than hue.
  - The legend inherits `--text` and `--text-sm`.
- Phone: one column. The segments fill the width, chips wrap, and the button
  is full width. The form takes about 260 px, so the top of the chart is in
  view on load.
- Desktop: the fieldsets sit in one wrapping row (Metric, Period, Lines),
  and the button follows at the end of the row or wraps under it.

### States

| State | What the owner sees |
|---|---|
| No readings in the period (every line empty) | A neutral notice in the chart card: "**No readings in this period.** Try a longer period or other rooms." No empty chart is drawn. |
| Store unavailable | The form still renders, because it comes from config. A warning notice replaces the chart: "**History unavailable.** The store did not respond. Try again in a minute." Status 502. |
| No rooms configured | The Lines group has only Outside. |
| JS disabled | The form works; the chart region shows nothing. This is acceptable and is not designed further. |

## Alerts `/alerts`

Content order: page header "Alerts" with "Updated HH:MM", the section
"Active" (`h2`, with the count, for example "Active (2)"), and the section
"Resolved in the last 30 days" (`h2`).

- Active: alert rows with `badge--alert`, subject, detail ("21.3 °C at last
  notification" when known), and "since …". They are ordered by onset, as
  today.
- Resolved: alert rows with `badge--neutral`, subject, and "start – end",
  newest first. These use the same component, not a table, because each
  entry is an event rather than a row of comparable values.
- Phone: rows wrap to two lines (design-system.md, Alert row). Desktop: one
  line per row, with the time right-aligned.

### States

| State | What the owner sees |
|---|---|
| No active alerts | A neutral notice: "No active alerts." |
| Nothing resolved | A neutral notice: "No alerts resolved in the last 30 days." |
| Alerter unavailable | Both sections are replaced by one warning notice: "**Alerts unavailable.** The alerter did not respond. This page retries every minute." Status 502, as today. |
| Unknown source | The subject shows the source id, as today. |

## Health `/health`

Content order: page header "Health", then the data table for the last 30 days,
newest first, with the caption "Daily health, last 30 days".

- Columns are unchanged: Date, Sleep, Sleep score, Resting HR (bpm), Steps,
  Body Battery max, Body Battery min.
- Phone: the table scrolls horizontally inside its wrapper, with Date as a
  sticky first column. The page itself never scrolls sideways.
- Desktop: the table fits within `--page-max`, with numbers right-aligned.
- The table has no chart. uPlot is not loaded on this page today, and the
  redesign does not add it.

### States

| State | What the owner sees |
|---|---|
| A value missing | "—" with a visually hidden "no data". |
| No data on any day | A neutral notice in place of the table: "**No health data in the last 30 days.** Check that the health collector is running." |
| Store unavailable | A warning notice: "**Health data unavailable.** The store did not respond. This page retries every 5 minutes." Status 502. |

## CSS delivery and templates

One `app.css`, inlined by a shared `base.html`; see ADR-0026.

## Acceptance checks for review

1. From each of the 4 pages, each other page is one nav activation away. The
   current page has `aria-current="page"`, the accent bar, and weight 600.
2. At 390 × 844 with 6 rooms and up to 2 active alerts, the alerts and every
   room value are visible without scrolling.
3. On the overview, the alerts block is before the room cards in both the DOM
   and the visual order. The rooms come before Outside.
4. With the alerter stopped, the overview still shows the rooms with the
   notice "Alerts unavailable." With the store stopped, every page renders
   the shell and a notice instead of plain text.
5. With the OS in dark mode, every page, the native controls, and the
   History chart's axes and lines use the dark tokens, without a reload of
   the chart's colours by hand.
6. Tab from page load: the first stop is "Skip to content", then the 4 nav
   links. Every focused element shows the 2 px accent ring.
7. No state is shown by colour alone. Every badge has text, and every chart
   line has a legend label and differs from its neighbour by dash or hue.
8. At 200 % zoom and at 320 px width, there is no horizontal page scroll. The
   Health table scrolls inside its region.
9. Page CSS has no raw colour, spacing, or font-size values outside the token
   block.
