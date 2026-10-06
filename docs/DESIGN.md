# WhisprCatch Design Brief

Single source of truth for **site/index.html** (web landing) and the **egui desktop app**
(theme.rs, settings_app.rs, wizard.rs, overlay.rs, tray menus). Every value here is a
decision, not a suggestion. If an implementer needs a value that isn't here, derive it
from the nearest token.

The two surfaces now speak **two deliberate languages**:

- **Website** — "**warm paper**": a cream editorial marketing surface, serif display type,
  deep-green section blocks, one mint accent. Confident and well-funded looking, aimed at
  people comparing us against paid dictation subscriptions (Part A below).
- **Desktop app** — "**black + orange**": a dark SaaS surface in the shadcn/ui manner.
  Neutral near-black, one orange accent, sidebar navigation, cards, a small component kit.
  Calm, dense where it matters, nothing decorative (Part B below). It replaced the
  mint "tactile engineer dark" language of `docs/DESIGN-handoff.md`, which stays archived.

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


# Part B — Desktop app ("black + orange")

Direction: a modern dark SaaS app (shadcn/ui, Linear, the Vercel dashboard). Neutral
near-black surfaces, **one orange accent**, generous whitespace, sentence-case copy.
**Dark only — there is no light theme and no theme picker.**

All tokens and components live in `apps/cli/src/theme.rs`. Screens never hand-pick a
colour or restyle a widget inline.

## B1. Type

Embedded in the binary (`apps/cli/assets/fonts/`, OFL — license alongside):

- Sans: **Geist** (Regular + Medium + SemiBold) — everything: UI text, headings, buttons.
- Mono: **Geist Mono** (Regular + Medium) — timestamps, key caps, numeric readouts, paths.

egui families: `Proportional` → Geist, `Monospace` → Geist Mono, plus named families
`GeistMedium` / `GeistSemiBold` / `GeistMonoMedium` (egui's `strong()` only recolours, so
weight = family switch via `theme::medium/semibold/mono_medium`). egui-phosphor (Regular)
is the icon set, appended to the font stack.

Scale: page title SemiBold 22 · card title SemiBold 15 · body 14 · secondary 12.5–13 ·
mono 11–12.5. Display headings (wizard) SemiBold 31, with the emphasised clause in
`ACCENT` ("Grant *access.*"). Uppercase only for mono machine readouts (`mono_upper`).

## B2. Palette (dark-only)

| Token          | Value     | Use |
|----------------|-----------|-----|
| `BG`           | `#0a0a0a` | Window background |
| `SIDEBAR`      | `#0e0e0e` | Navigation rail |
| `SURFACE`      | `#131313` | Cards |
| `SURFACE_2`    | `#1a1a1a` | Inputs, secondary buttons, selected nav item |
| `SURFACE_3`    | `#262626` | Hover/active fills, key caps, switch track (off) |
| `FG`           | `#fafafa` | Primary text |
| `TEXT_2`       | `#a3a3a3` | Secondary text |
| `MUTED`        | `#737373` | Labels, timestamps, descriptions |
| `BORDER`       | `#242424` | 1px hairlines everywhere |
| `RING`         | `#3c3c3c` | Hover and focus rings |
| `ACCENT`       | `#f97316` | Primary buttons, active nav, switches, granted, progress, recording |
| `ACCENT_HOVER` | `#fb8c3c` | Hovered primary button |
| `ON_ACCENT`    | `#140a02` | Text on an orange fill |
| `RED`          | `#ef4444` | Errors and destructive actions only |
| `AMBER`        | `#f59e0b` | Advisories that still work ("note:" problems, setup needed) |

`theme::tint(c)` ≈ 9% alpha (badge fills), `theme::tint_strong(c)` ≈ 18% (their rings).
Warnings sit on a neutral surface with an amber ring and icon, never on an amber fill:
amber over near-black reads as brown.

## B3. Radius, elevation, motion

- Radius: **6** (key caps) / **8** (buttons, inputs, nav items, list rows) / **12**
  (cards) / pill (badges, the overlay).
- Elevation is borders-first: `BG` → `SURFACE` → `SURFACE_2` + 1px `BORDER`. Only popups
  and menus get a (soft, black) shadow.
- Motion: switch 150ms, overlay expand 220ms cubic-out, LED pulse 2s, waveform eased
  toward the live mic level every frame. Nothing else animates.

## B4. Components (theme.rs)

- `button(ui, Variant, text)` / `button_with(ui, Variant, icon, text, small)` — shadcn
  `<Button>`: `Primary` (orange), `Secondary`, `Outline`, `Ghost`, `Destructive`. Height
  34 (`small` 28). Hover states belong to the variant. `primary_button` is the shorthand.
- `badge(ui, text, Tone)` — pill; `Neutral`, `Accent`, `Warn`, `Danger`.
- `kbd(ui, label)` — key cap: `SURFACE_3`, ring, darker bottom edge, mono, keeps the key's
  own case ("fn", "Right ⌘").
- `toggle(ui, &mut bool)` — switch: orange track when on, white thumb.
- `nav_item(ui, icon, label, selected)` — sidebar row; selected = `SURFACE_2` fill, orange
  icon, 2.5px orange rail.
- `card(ui)`, `card_header`, `page_header`, `section_label`, `progress`, `hairline`, `led`,
  `logo(ui, size)` (the app icon, `assets/icon-128.png`), `display`, `mono_upper`.

## B5. Surfaces

### Main window (`settings_app.rs`)
Opens at **1000×680**, centred, min 720×480; geometry is not persisted. A 232px
`SIDEBAR` rail: logo + name, then **Home · History · Text cleanup · Settings ·
Permissions · About** (an amber dot on Permissions while anything is missing), and a
status card pinned to the bottom ("Ready" / "Setup needed", "Hold ⟨kbd⟩ to dictate").
Content is a centred column ≤720px that scrolls, each page opening with `page_header`.
`--tab home|history|cleanup|settings|permissions|about` picks the page.

- **Home**: a "Finish setup" card while a permission is missing, three stat tiles,
  "How it works", the three latest dictations.
- **History**: search + list (selected row = orange rail) beside a detail card with
  metadata badges, Copy and Delete (inline confirm).
- **Text cleanup**: the transforms, Problems and the live Cleanup preview (removed words
  struck through in `RED`, added words in `ACCENT`), as in #49.
- **Settings**: Engine, Hotkey (with the fn notice below the picker), Output. Settings and
  Text cleanup share a sticky footer with **Save changes**.
- **Permissions**: the checklist below plus **Restart WhisprCatch**.

### Permission checklist (`permissions.rs`)
Shared by the wizard and Settings. One row per grant: icon plate (orange when granted),
title, one-line reason, and either a **Granted** badge or the one action that fixes it:

- **Microphone** — **Allow** shows the system prompt now, not mid-dictation.
- **Accessibility / Input Monitoring** — **Grant** registers the app with the system (so
  it is already in the list), opens the exact pane and starts the **drag helper**: a
  borderless AppKit panel pinned under the System Settings window, holding the app icon
  to drag into the list. Dragging adds the app and switches it on. It follows the
  Settings window, shows only while one is on screen, names the list that still needs
  the app, quits once both grants are in, and is never shown on Linux. One at a time
  (a lock file). The daemon also starts it whenever a grant is missing and System
  Settings is open, so it is there however Settings was reached (macOS's own "receive
  keystrokes" prompt opens Input Monitoring without our button). Closed with its ✕, it
  stays closed until System Settings closes.
- **fn key** (only when the hotkey is fn) — **Fix it** writes `AppleFnUsageType = 0`
  ("Press fn key to: Do Nothing"); Keyboard Settings is the fallback.

When the System Settings window that a row opened closes, the checklist's window comes
back to the front (`refocus_after_settings`). The app has no Dock icon, so macOS
otherwise hands focus to the next regular app and buries ours. So while the main window
or the wizard is open the process runs as a regular app (Dock icon), and the daemon drops
back to accessory when its tray starts. Both windows also raise themselves on their
first frame and after the Microphone prompt is answered.

### Pill overlay (`overlay.rs`)
One long-lived process for the daemon's whole life, driven over stdin (`show`,
`l <rms>`, `t`, `hide`). It sits bottom-centre on the display the pointer is on, 6pt
above the Dock, on every Space and over full-screen apps, never focused.

- **Idle**: a 40×9 near-black capsule with a light grey ring, always visible, so the user
  can see dictation is one key away.
- **Hover controls** (as Wispr Flow): under the pointer the capsule becomes a 72×32 mic
  button, with a 32pt round gear button 6pt to its right; both near-black with a faint
  ring, growing in over 180ms. The hovered one lightens and its icon turns `ACCENT`, and
  a label pill sits 8pt above it: **Dictate ⟨key⟩** (key in SemiBold) or **Settings**.
  Mic click: a hands-free dictation (a second click, or the hotkey, finishes it). Gear
  click: the main window on its Settings page. Never a system menu.
- **Listening**: expands to 92×30 with a centred 9-bar white waveform driven by the real
  mic level, newest level in the centre rippling outward. No LED. Hovered, a
  **Click to finish** label; a click finishes the dictation.
- **Transcribing**: three orange dots pulsing in sequence.

The egui window (248×96: the pill, the controls and a label) draws everything and stays
click-through. Clicks land on one AppKit non-activating panel (`overlay::mac::Hit`)
that never takes focus from the app being typed in, and that covers only what is
clickable now: a 64×22 patch on the idle capsule, the mic and gear once hovered, the pill
while listening, nothing while transcribing. It reports only enter, leave and click; the
overlay reads the pointer every frame while it is inside to know what is hovered. A
click that starts or finishes a dictation is written to the daemon as `toggle` on the
overlay's stdout.

### Tray / menu bar (`crates/tray`)
The app mark as a template image (`assets/icon-menubar.png`, from `icon-menubar.svg`),
so it follows light and dark menu bars. Menu: status header, Listening toggle,
**Open History** / **Preferences…**, divider, **Quit WhisprCatch**.

### Wizard (`wizard.rs`)
600×740 fixed, centred. Orange step dots, "STEP N OF 4" in mono, the logo on Welcome
and an orange stroke icon on a plate elsewhere (none on the permission step, which needs
the height), display title, one primary button pinned near the bottom. Welcome shows
the hotkey as a key cap; the permission step is the checklist above; Done says
**Start dictating** and, if the permission step was shown, relaunches the app so grants
given during setup apply to a fresh process.

## B6. Copy voice (app)

Same voice as the site: short, confident, privacy-forward, concrete numbers. Sentence
case everywhere; mono uppercase only for machine readouts. **No em dashes**, in log lines
and error strings too.

## B7. Capturing screenshots

The README and the website show real renders, not mockups. Dev-only hooks produce them;
none is reachable from normal use:

- `WC_SHOT=<path>` (+ `WC_SHOT_FRAMES`, default 30) saves a PNG of the window after N
  frames and exits — `apps/cli/src/shot.rs`.
- `WC_WIZARD_STEP=welcome|permission|download|done` opens the wizard on that step. The
  forced download step never fetches anything.
- `WC_OVERLAY=idle|listening|transcribing` pins the pill in one state with a synthetic
  waveform (`whisper-catch overlay`); `WC_OVERLAY_HOVER=mic|gear|pill` puts the pointer
  there (pill = the listening pill).
- `WC_DEMO_HISTORY=1` swaps the transcript log for a fixed sample set. **Always capture
  with this on.** Two sample rows carry a `raw`, so the cleanup preview has something
  real to replay; `every_demo_row_polishes_to_the_text_beside_it` keeps them honest.
- `WC_WINDOW=1440x900` opens the main window at that size, and `WC_SCROLL=<points>`
  opens the current page already scrolled.

Captures run against a throwaway `HOME`, with the models directory symlinked in so the
engine card reads Ready. `whisper-catch wizard` is a hidden subcommand that runs the
wizard on its own. Published files live in `docs/screenshots/`.
