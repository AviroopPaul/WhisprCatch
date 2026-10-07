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

# Part A — Website ("black + orange")

Tokens live in **`site/assets/site.css`** `:root` and are shared by every page under `site/`.
Pages never hand-pick a colour. `docs/og-card.html` mirrors the same tokens; render it at
1200x630 into `site/assets/og-card.png`. The site is dark only (`color-scheme: dark`,
`theme-color` `#0a0a0a`) and derives from the app palette in Part B.

## A1. Typography

Three hosted families via a single Google Fonts `<link>`:

- **Newsreader** (400, roman + italic, variable optical size) — all display type. The
  italic carries the emphasis in every headline ("You talk. *It types.*").
- **Geist** (400/500/600/700) — UI and body text.
- **Geist Mono** (400/500) — commands, terminal blocks, numeric readouts.

`kbd` uses `system-ui` first, because Geist Mono has no ⌘ or ⌥ glyph.

| Token      | Size (px)               | Family    | Weight | Line height | Tracking            | Use |
|------------|-------------------------|-----------|--------|-------------|---------------------|-----|
| `display`  | clamp(46, 8.4vw, 108)   | Newsreader| 400    | 0.95        | -0.032em            | Hero h1, closer |
| `h2`       | clamp(33, 5vw, 60)      | Newsreader| 400    | 1.03        | -0.026em            | Section titles |
| `h3`       | clamp(23, 2.3vw, 28)    | Newsreader| 400    | 1.16        | -0.018em            | Card titles |
| `body-lg`  | clamp(17, 1.55vw, 20.5) | Geist     | 400    | 1.55        | 0                   | Section intros (`.lede`) |
| `body`     | 16.5                    | Geist     | 400    | 1.62        | 0                   | Default |
| `small`    | 13.5                    | Geist     | 400    | 1.5         | 0                   | Captions, footnotes |
| `eyebrow`  | 12                      | Geist     | 600    | 1           | +0.15em, uppercase  | Kicker above every h2 |
| `mono`     | 13–14                   | Geist Mono| 400    | 1.9         | 0                   | Commands, terminal |

## A2. Colour

| Token         | Value                    | Use |
|---------------|--------------------------|-----|
| `bg`          | `#0a0a0a`                | Page canvas |
| `sheet`       | `#121212`                | Cards, table, glow bands, footer |
| `panel`       | `#181818`                | Alternate bands, table head, kbd |
| `surface-2/3` | `#1a1a1a` / `#262626`    | Cards inside a glow band, active segment |
| `fg`          | `#fafafa`                | Primary text |
| `fg-2`        | `#a3a3a3`                | Secondary text |
| `muted`       | `#8c8c8c`                | Eyebrows, captions (app MUTED `#737373` is lifted to keep 4.5:1) |
| `border`      | `#242424`                | 1px hairlines |
| `ring`        | `#3c3c3c`                | Hovered borders, dividers inside glow bands |
| `accent`      | `#f97316`                | Button fill, ticks, links, focus ring, hero ribbon |
| `accent-hover`| `#fb8c3c`                | Button hover |
| `on-accent`   | `#140a02`                | Text on accent fills |
| `accent-glow/soft/line` | orange at .22 / .12 / .34 | Hero bloom, tinted cells, accent borders |
| `red`         | `#ef4444`                | The price you are *not* paying |
| `amber`       | `#f59e0b`                | Reserved |

Rules: orange fills buttons and small marks, and carries accent text; never large areas
(the one exception is the hero ribbon band). `band-glow` sections (privacy, and the
closer on the comparison page) are `sheet` with an orange radial glow from the top edge.

## A2b. Page and hero

Home page order: hero, demo video, how it works, privacy, comparison table, install, FAQ,
footer. No calculators, marquees or app-logo strips; the site shows less, not more.

The hero ribbon (`site/assets/ribbon.js`, `.ribbon` in `site.css`) is inline SVG drawn
from the stage size: raw speech (`muted` grey text on a looping path) flows into the
Catcher pill (13 seeded waveform bars), and clean text (`on-accent` on an `accent` band)
flows out. It pauses off screen and with the tab hidden, and draws one static frame under
`prefers-reduced-motion`. Keep it decorative (`aria-hidden`).

## A3. Geometry, elevation, motion

- Content column `1140px`, prose/FAQ column `800px`, page padding `24px`.
- Section rhythm: `clamp(72px, 9.5vw, 132px)` top and bottom.
- Radius: `8` chips · `12` command chips · `18` · `26` cards · `34` big cards · `999` pills.
- Elevation on dark is a 1px top hairline plus an orange glow (`--sh-1/2/3`), strongest
  under the hero media. Never a drop shadow.
- Motion: 16px rise + fade on scroll (`.reveal`, 0.7s) and the hero ribbon. Everything
  collapses under `prefers-reduced-motion`.

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
- Serif: **Newsreader** Regular — display headlines only (hero banner, Settings titles), via `theme::serif(size)`.
- Mono: **Geist Mono** (Regular + Medium) — timestamps, key caps, numeric readouts, paths.

egui families: `Proportional` → Geist, `Monospace` → Geist Mono, plus named families
`GeistMedium` / `GeistSemiBold` / `GeistMonoMedium` (egui's `strong()` only recolours, so
weight = family switch via `theme::medium/semibold/mono_medium`). egui-phosphor (Regular)
is the icon set, appended to the font stack.

Scale: page title SemiBold 28 · hero / Settings title Newsreader 36 · card title SemiBold 15 · body 14 · secondary 12.5–13 ·
mono 11–12.5. Display headings (wizard) SemiBold 31, with the emphasised clause in
`ACCENT` ("Grant *access.*"). Uppercase only for mono machine readouts (`mono_upper`).

## B2. Palette (dark-only)

| Token          | Value     | Use |
|----------------|-----------|-----|
| `BG`           | `#0a0a0a` | Window background |
| `SIDEBAR`      | `#0e0e0e` | Settings modal's section column |
| `SHEET`        | `#121212` | The rounded content sheet pages sit on, the modal's right pane |
| `PANEL`        | `#181818` | Cards inside the sheet (`card`, `group`) |
| `SURFACE`      | `#131313` | Nav hover fill, popups |
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
| `SCRIM`        | black, alpha 215 | Behind the Settings modal |

`theme::heat(level)` is the heatmap ramp: 0 = `SURFACE_3`, 1 to 4 = `ACCENT` mixed toward
`PANEL` (`theme::mix`) at 30/52/76/100%.

`theme::tint(c)` ≈ 9% alpha (badge fills), `theme::tint_strong(c)` ≈ 18% (their rings).
Warnings sit on a neutral surface with an amber ring and icon, never on an amber fill:
amber over near-black reads as brown.

## B3. Radius, elevation, motion

- Radius: **6** (key caps) / **8** (buttons, inputs, nav items, list rows) / **12**
  (cards) / pill (badges, the overlay).
- Elevation is borders-first: `BG` → `SHEET` → `PANEL` → `SURFACE_2` + 1px `BORDER`. The sheet is radius 14, the hero 16, the Settings modal 16. Only popups
  and menus get a (soft, black) shadow.
- Motion: switch 150ms, overlay expand 220ms cubic-out, LED pulse 2s, waveform eased
  toward the live mic level every frame. Nothing else animates.

## B4. Components (theme.rs)

- `button(ui, Variant, text)` / `button_with(ui, Variant, icon, text, small)` — shadcn
  `<Button>`: `Primary` (orange), `Secondary`, `Outline`, `Ghost`, `Destructive`, `Light` (light fill, dark text: on the hero). Height
  34 (`small` 28). Hover states belong to the variant. `primary_button` is the shorthand.
- `badge(ui, text, Tone)` — pill; `Neutral`, `Accent`, `Warn`, `Danger`.
- `kbd(ui, label)` — key cap: `SURFACE_3`, ring, darker bottom edge, mono, keeps the key's
  own case ("fn", "Right ⌘").
- `toggle(ui, &mut bool)` — switch: orange track when on, white thumb.
- `nav_item(ui, icon, label, selected)` — sidebar row, 38px: 18px icon, medium 14.5 label,
  radius 8; selected = `SURFACE_2` fill, orange icon, `FG` label.
  `nav_item_with_badge(.., Some("New"))` adds an orange badge on the right edge.
- `hero(ui, headline, body, |ui| actions)` — banner, radius 16: `PANEL` to a warm `ACCENT`
  glow (vertex-coloured mesh, tokens only), serif 36 headline, `TEXT_2` body, buttons below.
- `group(ui, |ui| ..)` + `row(ui, title, desc, |ui| control)` — settings rows in one `PANEL`
  card, a hairline between rows; title medium 15, description 13.5 `TEXT_2`, control right.
- `page_header_with(ui, title, badge, |ui| controls)` — title, optional badge and
  right-aligned controls on one line.
- `card(ui)`, `card_header`, `page_header`, `section_label`, `progress`, `hairline`, `led`,
  `logo(ui, size)` (the app icon, `assets/icon-128.png`), `display`, `mono_upper`.

## B5. Surfaces

### Main window (`settings_app.rs`)
Opens at **1000×680**, centred, min 720×480; geometry is not persisted. On macOS there is
no title bar: the traffic lights sit over the UI (`with_fullsize_content_view`), the sidebar
leaves a 44px top inset for them, and the empty top strip (sidebar top, 10px gap above the
sheet) drags the window and zooms on double-click. Linux keeps the system title bar. Window ground and
the 232px sidebar are `BG`, with no line between them. Content sits in a rounded **sheet**
(`SHEET`, radius 14, 1px `BORDER`, inset 10px from the top, right and bottom). Pages are a
centred column ≤980px with 40px side padding that scrolls, each opening with `page_header`.

Sidebar: logo 26 + name (SemiBold 18); **Home · Insights · History · Notes** (orange "New"
badge) **· Text cleanup**; at the bottom the status
area (one quiet "Ready" line, or a "Setup needed" card listing only the grants still
missing with a **Finish setup** button that opens the modal on Permissions), a hairline, then **Settings**
(opens the modal; amber dot while a permission is missing) and **About**.
`--tab home|insights|history|notes|cleanup|about` picks the page; `settings|general|catcher|system|
permissions` open the Settings modal on that section (`parse_tab`).

- **Home**: page header, the hero ("Hold fn and talk. Your words land where you type."),
  then Recent dictations with **View all**.
- **Insights** (`settings_app/insights_page.rs`, numbers from `insights.rs`): three stat
  cards (words per minute with a painted semicircle gauge on a 200 wpm scale and "Nx faster
  than typing" at 40 wpm; minutes saved, dictations, cleaned up; total words, this week,
  speaking time), then **App usage** (up to 6 apps, `ACCENT` bars scaled to the top app,
  a tinted chip for small shares, mono "N WORDS · APP"; no-app dictations read "Other")
  and the **streak** card (7 x 20 heatmap, Sunday first, hover tooltip, Less/More legend).
  Cards stack below 640px; a title's mono readout drops under it in narrow cards. Stats are
  computed once on open and on reload. A "History is off" notice opens Settings when
  `cfg.history` is false. Under `WC_DEMO_HISTORY` "today" is fixed to the demo day.
- **Notes** (`settings_app/notes_page.rs`, storage `notes.rs`, editor `notes_ui.rs`): header
  with a "Beta" badge and, right-aligned, **Add to Catcher** + info tip + toggle (saves
  `catcher_notes` at once, touching no other field); the hero ("Catch a thought before it
  slips away", **Start new note**); **Recents** with search / new / refresh icons, rows
  of title, one-line preview and `list_time`; "No notes yet" when empty. A row opens the
  editor view (back arrow, title, the editor in a `PANEL` card). Notes are plain `.md`
  files, `<unix_millis>.md` in `<data_dir>/whisper-catch/notes/`; an empty note leaves
  no file. The editor autosaves 600ms after the last key and on close, shows
  Saved / Saving, and has Show in Finder and Delete (inline confirm, no dialog). The list
  reloads on open, on refresh and every 3s while shown. `WC_NOTE=<id>` opens a note in
  the editor for captures.
- **History**: search + list (selected row = orange rail) beside a detail card with
  metadata badges, Copy and Delete (inline confirm).
- **Text cleanup**: the transforms, Problems and the live Cleanup preview (removed words
  struck through in `RED`, added words in `ACCENT`), as in #49. Keeps its own sticky
  **Save changes** footer.
- **Settings modal**: `SCRIM` over the whole window; a centred panel at 88% of the window
  (max 1120×780, radius 16). Left column (240px, `SIDEBAR`): uppercase "SETTINGS",
  sections with icons, the version at the bottom. Right pane: serif title, grouped rows,
  a × (also Esc or a click on the scrim closes it) and a **Save changes** bar. Sections:
  **General** (Shortcut, Speech model and its download state, Live typing, Keep history),
  **Catcher** (Show Catcher, Notes button), **System** (Launch at login, the fn notice, where data
  lives with Open folder buttons), **Permissions** (the checklist below plus **Restart
  WhisprCatch**; no save bar).
- **About**: version, links, privacy.

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

### Catcher (`overlay.rs`)
The capsule is called the **Catcher** in all user-facing text; code names (`overlay`) stay.

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
- **Notes button** (off by default, `catcher_notes`; toggled on the Notes page or in
  Settings › Catcher): a second round 32pt button between the mic and the gear
  (`NOTE_PENCIL`, label **New note**); the gear moves out one slot. A click runs
  `whisper-catch note`, the quick note window (see below). The Catcher re-reads `config.toml` every 2s when its
  mtime moved, so the toggle applies live. Off, the Catcher is unchanged.
- **Listening**: expands to 92×30 with a centred 9-bar white waveform driven by the real
  mic level, newest level in the centre rippling outward. No LED. Hovered, a
  **Click to finish** label; a click finishes the dictation.
- **Transcribing**: three orange dots pulsing in sequence.

The egui window (248×96: the pill, the controls and a label) draws everything and stays
click-through. Clicks land on one AppKit non-activating panel (`overlay::mac::Hit`)
that never takes focus from the app being typed in, and that covers only what is
clickable now: a 64×22 patch on the idle capsule, the mic, Notes button and gear once hovered, the pill
while listening, nothing while transcribing. It reports only enter, leave and click; the
overlay reads the pointer every frame while it is inside to know what is hovered. A
click that starts or finishes a dictation is written to the daemon as `toggle` on the
overlay's stdout.

### Quick note window (`note_window.rs`)
Opened by `whisper-catch note [--id]`; one at a time (temp-dir lock). No system title bar
(`with_decorations(false)`, transparent): it paints its own body, radius 16, `BG` fill,
1px `BORDER`, no shadow. 530×430, always on top, the editor focused so held-fn dictation
types straight in. Captures of the transparent window show black corners.
- **Top bar** (48, drags the window): logo 24; one tab per note opened in this session
  (title or "Untitled", `ACCENT` 2px underline and FG text when active, `TEXT_2` otherwise,
  × saves and closes the tab); **+** for a new note; right-aligned expand (toggles
  530×430 and 860×620) and ×. Esc or × saves everything and closes.
- **Sidebar** (170, or a 52 icon rail; the collapsed state lives in egui memory, not on
  disk): Collapse notes, New note, Search notes, hairline, recent notes (click opens a
  tab or focuses it), and Text cleanup at the bottom (opens `settings --tab cleanup`).
  The rail has the same actions as icons with tooltips; its search expands the sidebar and
  focuses the field.
- **Editor card** (`PANEL`, radius 14, 8px inset): borderless text at 15.5 with the
  `ACCENT` caret. Empty, it shows a keycap with the hotkey and "to dictate" in `MUTED`.
  A floating **Copy** pill (`Variant::Light`, becomes "Copied" for 1.5s) appears once the
  note has text. Autosave and the no-empty-file rule are as on the Notes page; Delete
  and Show in Finder live only there.
- Dev hooks: `WC_NOTE_TABS=id,id` opens extra tabs, `WC_NOTE_COLLAPSED=1` starts collapsed.

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
  waveform (`whisper-catch overlay`); `WC_OVERLAY_HOVER=mic|notes|gear|pill` puts the pointer
  there (pill = the listening pill; `notes` also turns the Notes button on). The quick
  note window is `whisper-catch note [--id <id>]` with `WC_SHOT`.
- `WC_DEMO_HISTORY=1` swaps the transcript log for a fixed sample set. **Always capture
  with this on.** Two sample rows carry a `raw`, so the cleanup preview has something
  real to replay; `every_demo_row_polishes_to_the_text_beside_it` keeps them honest.
  The sample set also has an app name per row and a 12-week backlog (a streak up to the
  demo day) so Insights has something to show.
- `WC_PERMS_OK=1` fakes "every permission granted", to capture the sidebar's Ready state.
- `WC_WINDOW=1440x900` opens the main window at that size, and `WC_SCROLL=<points>`
  opens the current page already scrolled.

Captures run against a throwaway `HOME`, with the models directory symlinked in so the
engine card reads Ready. `whisper-catch wizard` is a hidden subcommand that runs the
wizard on its own. Published files live in `docs/screenshots/`.
