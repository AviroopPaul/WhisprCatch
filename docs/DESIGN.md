# WhisprCatch Design Brief

Single source of truth for **site/index.html** (web landing) and the **egui desktop app**
(theme.rs, settings_app.rs, wizard.rs, overlay.rs, tray menus). Every value here is a
decision, not a suggestion. If an implementer needs a value that isn't here, derive it
from the nearest token.

The two surfaces now speak **two deliberate languages**:

- **Website** — "**warm paper**": a cream editorial marketing surface, serif display type,
  deep-green section blocks, one mint accent. Confident and well-funded looking, aimed at
  people comparing us against paid dictation subscriptions (Part A below).
- **Desktop app** — "**tactile engineer dark**", adopted from the EchoNode design handoff
  (archived verbatim at `docs/DESIGN-handoff.md`): precise, developer-native,
  keyboard-first. The app should feel like a hardware push-to-talk radio: status LEDs,
  signal meters, mono uppercase labels (Part B below).

---

# Part A — Website ("warm paper")

Tokens live in **`site/assets/site.css`** and are shared by every page under `site/`.
Pages never hand-pick a colour. `docs/og-card.html` mirrors the same tokens.

## A1. Typography

Three hosted families via a single Google Fonts `<link>`:

- **Newsreader** (400, roman + italic, variable optical size) — all display type. The
  italic carries the emphasis in every headline ("You talk. *It types.*").
- **Figtree** (400/500/600/700) — UI and body text.
- **Fragment Mono** (400) — commands, terminal blocks, numeric readouts.

`kbd` uses `system-ui` first, because Fragment Mono has no ⌘ or ⌥ glyph.

| Token      | Size (px)               | Family    | Weight | Line height | Tracking            | Use |
|------------|-------------------------|-----------|--------|-------------|---------------------|-----|
| `display`  | clamp(46, 8.4vw, 108)   | Newsreader| 400    | 0.95        | -0.032em            | Hero h1, closer |
| `h2`       | clamp(33, 5vw, 60)      | Newsreader| 400    | 1.03        | -0.026em            | Section titles |
| `h3`       | clamp(23, 2.3vw, 28)    | Newsreader| 400    | 1.16        | -0.018em            | Card titles |
| `body-lg`  | clamp(17, 1.55vw, 20.5) | Figtree   | 400    | 1.55        | 0                   | Section intros (`.lede`) |
| `body`     | 16.5                    | Figtree   | 400    | 1.62        | 0                   | Default |
| `small`    | 13.5                    | Figtree   | 400    | 1.5         | 0                   | Captions, footnotes |
| `eyebrow`  | 12                      | Figtree   | 600    | 1           | +0.15em, uppercase  | Kicker above every h2 |
| `mono`     | 13–14                   | Fragment  | 400    | 1.9         | 0                   | Commands, terminal |

## A2. Colour

Warm cream canvas, near-black ink, deep green for full-bleed blocks, one mint accent.

| Token         | Hex / value              | Use |
|---------------|--------------------------|-----|
| `paper`       | `#fcfbec`                | Page canvas |
| `paper-2`     | `#fffef7`                | Raised cards, nav, command chips |
| `paper-3`     | `#f3f1de`               | Alternate bands, table head, footer |
| `ink`         | `#16191b`                | Primary text |
| `ink-2`       | `#545b57`                | Secondary text |
| `ink-3`       | `#878d86`                | Eyebrows, captions, muted |
| `rule`        | `rgba(22,25,27,.12)`     | 1px hairlines |
| `rule-2`      | `rgba(22,25,27,.22)`     | Hovered borders, strikethroughs |
| `forest`      | `#063c34`                | Full-bleed dark sections, dark buttons |
| `forest-2`    | `#0a4f44`                | Cards inside a forest section |
| `on-forest`   | `#eaf6f2`                | Text on forest |
| `mint`        | `#5de8cd`                | Primary button fill, ticks, LED bars |
| `on-mint`     | `#06342c`                | Text on mint fills |
| `ember`       | `#e4572e`                | The price you are *not* paying, recording LED |
| `butter`      | `#ffc96b`                | Highlighter marks (sparingly) |

Rules: mint fills buttons and small marks only, never large areas. Forest sections always
carry a serif headline. No gradients except the single mint bloom behind the hero.

The one exception to "never hand-pick a colour" is third-party app marks. They live in an
inline `<symbol>` sprite at the top of the page, are referenced with `<use href="#i-name">`,
and take their brand hex from an inline `style="color:…"` on the `<svg>`. Those hexes belong
to their owners, so they are not tokens and must not be reused for anything else. The marks
are shown to say where WhisprCatch types, nothing more.

## A3. Geometry, elevation, motion

- Content column `1140px`, prose/FAQ column `800px`, page padding `24px`.
- Section rhythm: `clamp(72px, 9.5vw, 132px)` top and bottom.
- Radius: `8` chips · `12` command chips · `18` · `26` cards · `34` big cards · `999` pills.
- Elevation is three warm shadows (`--sh-1/2/3`), softest on cards, deepest under the hero
  media. Never a hard black shadow.
- Motion: 16px rise + fade on scroll (`.reveal`, 0.7s), 38–46s linear marquees, 2s LED
  pulse. Everything collapses under `prefers-reduced-motion`.

## A4. Copy voice

Short declaratives, contractions, concrete numbers. **No em dashes.** One dry joke per
section is allowed; the footer keeps its quirk line. State what the app cannot do as
plainly as what it can, and never let a claim outrun the code.

## A5. SEO surface

Every page carries: one `h1`, keyword-bearing `h2`s, canonical URL, Open Graph and Twitter
cards, and a JSON-LD `@graph`. `FAQPage` answers must match the visible `<details>` text
word for word. `site/llms.txt` and `site/llms-full.txt` are the machine-readable summary
for answer engines; `robots.txt` names the AI crawlers explicitly.

---

# Part B — Desktop app ("tactile engineer dark")

Direction: precise, developer-native, keyboard-first. A hardware push-to-talk radio:
physical button, status LED, clean signal meter. **Dark only — there is no light theme
and no theme picker.** Source: `docs/DESIGN-handoff.md` §2–3 (surface composition and
design system); adapted here to WhisprCatch's real feature set.

All tokens live in `apps/cli/src/theme.rs`. Screens never hand-pick colors.

## B1. Type

Embedded in the binary (`apps/cli/assets/fonts/`, OFL — license alongside):

- Sans: **Geist** (Regular + Medium + SemiBold) — UI text, labels, buttons, titles.
- Mono: **Geist Mono** (Regular + Medium) — timestamps, hotkey chips, section labels,
  numeric readouts, paths.
- Serif: **Newsreader** (Regular + Italic) — display type only: wizard step titles, the
  history empty state, and the transcript body in the detail pane. The same face the
  website sets its headlines in, and used the same way: roman then italic for the
  emphasised clause ("Three *permissions.*"). Never for UI chrome.

egui families: `Proportional` → Geist, `Monospace` → Geist Mono, plus named families
`GeistMedium` / `GeistSemiBold` / `GeistMonoMedium` (egui's `strong()` only recolors, so
weight = family switch via `theme::medium/semibold/mono_medium`). egui-phosphor
(Regular) is appended for icons — used sparingly, muted.

Text scale: Body/Button 14 · Small 11.5 · Mono 12 · section labels mono 11 uppercase ·
wizard titles 23 SemiBold. Hierarchy comes from weight + muted color, never from many
sizes on one screen. **Anything uppercase is mono** (`theme::mono_upper`,
`theme::section_label`).

## B2. Palette (dark-only)

Warm near-black neutrals — a dark cousin of the website's cream, not pure zinc —
plus the site's mint as the one accent and two signal colors. Signal colors mean
state, never decoration.

| Token       | Value       | Use |
|-------------|-------------|-----|
| `BG`        | `#0b0d0c`   | Window background |
| `SURFACE`   | `#141817`   | Cards, selected list rows, icon plates |
| `SURFACE_2` | `#1c2120`   | Buttons, inputs, raised controls |
| `SURFACE_3` | `#262c2a`   | Hover/active fills, toggle troughs |
| `FG`        | `#e9efec`   | Primary text |
| `TEXT_2`    | `#9aa5a0`   | Secondary text |
| `MUTED`     | `#6e7873`   | Labels, timestamps, metadata |
| `BORDER`    | `#1e2322`   | 1px hairlines everywhere |
| `RING`      | `#333b39`   | Focus/selected/hover rings |
| `MINT`      | `#5de8cd`   | Primary button fill, ready/active, ticks, selected-row rail |
| `ON_MINT`   | `#06342c`   | Text on a mint fill |
| `RED`       | `#ef5f52`   | Recording (LED, waveform, destructive) |
| `AMBER`     | `#f0a94c`   | Processing (spinner, dots) + hotkey chips |

`MINT` is the website's accent, unchanged. That single shared value is what makes
the app and the landing page read as one product.

`theme::tint(color)` = ~9% alpha, for chip fills behind signal text.
`theme::tint_strong(color)` = ~18%, for the ring around a tinted plate.
The primary button is a `MINT` fill with `ON_MINT` text — the site's CTA, in the dark.

## B3. Radius, elevation, motion

- Radius: **4** (chips) / **6** (buttons, inputs, list rows) / **10** (cards) /
  **14** (windows). Pill overlay is fully rounded.
- Elevation is borders-first: background step (`BG` → `SURFACE` → `SURFACE_2`) + 1px
  `BORDER`. No drop shadows inside windows.
- Motion: LED pulse 2s ease-in-out (opacity 1 → 0.4 → 1), spinner 1s linear,
  150–200ms color transitions. Nothing else animates.

## B4. Components (theme.rs)

- `led(ui, color, pulse)` — status LED with soft halo.
- `key_chip(ui, label)` — hotkey chip: amber mono uppercase on amber tint, radius 4.
- `section_label(ui, text)` — mono uppercase muted 11px heading.
- `mono_upper(text, size, color)` — mono uppercase micro-text (timestamps, readouts).
- `card(ui)` — SURFACE fill, hairline ring, radius 10, 16px inset.
- `toggle(ui, &mut bool)` — hardware-style switch, green when on.
- `primary_button(ui, text)` — the one high-emphasis action per screen.

## B5. Surfaces

### Main window (`settings_app.rs`)
Opens at **1000×680**, centered, min 720×480. Not maximized and geometry is not
persisted: this is a utility window, and a remembered 27-inch frame is mostly
background. macOS hands a window back at its own size regardless, so both windows
assert their size once from inside `update()` on the first frame. Header (52px): green LED + "WhisprCatch" left · **top-center segmented
control** (History | Settings) · mono stats readout right ("163 WORDS · 9 UTT · 1 MIN").

- **History**: left sidebar (288px) = search field + chronological list. Row = mono
  uppercase muted timestamp ("TODAY 23:21") + right-aligned mono duration + 2-line
  clamped preview; selected = SURFACE fill + RING ring. Footer = mono count + quiet
  "Clear all" with inline red confirm. Right pane = mono timestamp + metadata readout
  ("6.1S SPOKEN · 19 WORDS · 0.38S INFERENCE"), ghost Copy/Delete top-right (delete
  confirms inline, red), hairline, then the transcript at 15px in a ≤720px column.
  Empty state: muted mic glyph on a surface plate, "No transcripts yet", "Hold
  ⟨key chip⟩ and speak to dictate." with the *configured* hotkey.
- **Settings**: centered 560px column of sections, each = mono uppercase label + card:
  **ENGINE PARAMETERS** (model picker, mono RAM/download readout, green READY LED or
  green progress bar), **HOTKEY** (key picker + amber key chip preview), **OUTPUT
  BEHAVIOR** (green toggles: live typing, recording indicator, keep history, start on
  login), **ABOUT** (version, links, config path in mono). One primary Save button.

### Pill overlay (`overlay.rs`)
232×40, bottom-center, dark translucent (zinc-950 @ ~92%) with a subtle white ring,
fully rounded, click-through, never takes focus.

- **Listening**: red pulsing LED (2s) + 4-bar red waveform + "Listening…" + elapsed
  mono timer right-aligned behind a vertical hairline.
- **Transcribing**: amber arc spinner (1s) + "Transcribing…" + 3 amber dots pulsing
  sequentially behind the hairline.

### Tray / menu bar (`crates/tray`)
Native menus can't be themed; the language shows in structure + icon states.
Menu: status header (state — model, "Hold ⟨key⟩ to dictate", disabled rows) ·
Listening toggle · **Open History** / **Preferences…** (opens the Settings tab) ·
divider · **Quit WhisprCatch**. Icons: idle = outline/template mic, recording = red,
muted = crossed mic (Linux icon names; macOS uses a template glyph).

### Wizard (`wizard.rs`)
560×640 fixed, centered, non-resizable. Green step dots (done fill / current ring), "STEP N OF 4" in mono
uppercase, painted stroke icon on a surface plate, SemiBold 23 title, green mono
privacy chip, green download progress bar with mono readout, amber spinner while
waiting on authorization, amber key chip on the done screen. One primary button,
pinned near the bottom.

## B6. Copy voice (app)

Same voice as the site: short, confident, privacy-forward, concrete numbers
("0.38S INFERENCE", not "blazingly fast"). Mono uppercase for machine facts, sentence
case for human sentences. Quirk allowed once per surface (wizard done-screen).
**No em dashes**, same as Part A — that applies to log lines and error strings too,
not just what's on screen.


## B7. Capturing screenshots

The README and the website show real renders, not mockups. Three dev-only hooks
produce them; none is reachable from normal use:

- `WC_SHOT=<path>` (+ `WC_SHOT_FRAMES`, default 30) saves a PNG of the window after
  N frames and exits — `apps/cli/src/shot.rs`.
- `WC_WIZARD_STEP=welcome|permission|download|done` opens the wizard on that step, so
  captures don't depend on real permission or model state. The forced download step
  never fetches anything.
- `WC_DEMO_HISTORY=1` swaps the transcript log for a fixed sample set. **Always capture
  with this on** — the history pane otherwise shows whatever the person running the
  capture actually dictated.

`whisper-catch wizard` is a hidden subcommand that runs the wizard on its own.
Published files live in `docs/screenshots/` (full size, for the README) and
`site/assets/` (resized, for the landing page).
