# Omacus design language

One look for the desktop app, the web app at `app.omacus.com`, and the website. Calm, precise, and quiet enough that the page someone is writing stays in front.

Screenshots of the pass that introduced this guide live in [`docs/design/`](design/): `before-*` and `after-*` for the desktop editor, settings, about, and the replace dialog, in light and dark; `after-picker-*` and `after-hover-*` for the autocomplete popup and a value card; `web-settings-*`, `web-about-dark`, and `web-home` for the web shell and the site.

## Where the tokens live

The code is the source of truth. The guide describes it. If they disagree, change the guide or the tokens in the same commit.

| Surface | Source |
| --- | --- |
| Desktop colors, type, spacing, radii | `src/theme.rs` (`Theme`, `token`, the border mix, danger and warning) |
| Desktop frames, buttons, switches, egui visuals | `src/design.rs` |
| Website and web shell | `site/css/tokens.css`, imported first by `site/css/site.css` |
| Which app theme is on | `documentElement.dataset.appTheme` (`light`, `dark`, `paper`, `dusk`) |

`site/app/app.js` does not set color literals. It sets `data-app-theme`, and the browser resolves `--app-bg` for the theme-color of the chrome. Built-in palettes in `tokens.css` are checked against `src/theme.rs` by `tests/check_design.py` (borders included). A hardcoded `Color32::from_rgb(…)` or `hex("#…")` outside `src/theme.rs`, or a hex color in any other file under `site/**/*.css`, fails CI. A justified exception is a `design-allow` comment on that line, or a path in `tests/design-allow.txt`.

A swatch painted from a calculated color (`Color32::from_rgb(r, g, b)` with variables) is not a token. Leave it.

The marketing site keeps its own gold on near-black (`--accent`, `--bg`). That palette is not an app theme. The homepage demo uses the real dark app theme (`--app-*` on `:root`), so a screenshot of the product matches the product.

## Principles

**Calm.** One accent. Secondary text is quieter than the page, never a second brand color. Borders are a mix of text and background, not the selection color.

**Premium.** Aligned type, an 4px grid, one radius scale, one shadow. Nothing is outlined in the selection wash.

**Private.** The interface does not ask for an account, and it does not decorate itself with other people's data. Empty states and About say, in plain words, that pages stay on this computer.

**Fast.** The desktop app paints the next frame. It does not animate layout. The web shell may fade a color in 120ms. The editor, the answer column, and dragging a page do not animate.

**Content first.** The page is the product. Chrome (sidebar, toolbar, settings) gets out of the way. Answers use the accent so they read as results, not as another sentence.

## Voice and microcopy

Plain words. Sentence case. Say "page", not "sheet", in anything a person reads.

- Buttons are verbs: "Save", "Choose folder…", "Delete". An ellipsis means another step, not a pause.
- One short description under a setting. It says what the choice does, not how the widget works.
- Errors say what happened and, when we know, what to do. No codes, no "oops".
- Don't invent section labels in all caps. "Pages" and "Recently deleted", not "PAGES".
- Don't use "simply", "just", or product jargon ("surface", "token", "modal") in the interface. Those words belong in this guide.

## Color

Roles, not one-off hex values. Each built-in theme defines the role. Custom and Omarchy themes derive the same roles from the file they load: `text_secondary` and `text_muted` mix toward the background, `border` uses the mix constants below.

| Role | Rust | CSS | Use |
| --- | --- | --- | --- |
| Background | `background` | `--app-bg` | The page and the main panel |
| Surface | `surface()` / `sidebar` | `--app-side` | Sidebar, dialogs, popups, setting cards |
| Border | `border()` | `--app-line` | Hairline around cards, fields, menus |
| Border, strong | `border_strong()` | `--app-line-strong` | Hovered controls, separators that must show |
| Text | `text()` / `foreground` | `--app-fg` | Titles and body |
| Text, secondary | `text_secondary()` / `dim` | `--app-dim` | Descriptions, idle sidebar rows |
| Text, muted | `text_muted()` / `comment` | `--app-comment` | Section labels, hints, comments in the page |
| Accent | `accent` | `--app-accent` | Answers, the cursor, primary buttons, links, focus |
| On accent | `on_accent()` | `--app-on-accent` | Label on a primary button |
| Selection | `selection` | `--app-sel` | The open page, a selected menu row |
| Hover | `hover` | `--app-hover` | Row and button hover |
| Danger | `danger()` | `--app-danger` | Errors, destructive buttons |
| Warning | `warning()` | `--app-warning` | A warning that is not a failure |
| Answer | `accent` | `--app-accent` | A result. Dim it when it is stale or just copied |

Syntax colors (`keyword`, `variable`, `defined`, `number`) stay inside the editor. They are not UI chrome. `defined`, `variable`, and `keyword` are three different colors.

### Built-in themes

Backgrounds are pinned. Light `#ffffff`, dark `#1a1b26`, paper `#f6f1e7`, dusk `#1c2433`.

Text, secondary text, muted text, the accent, danger, and warning each clear WCAG AA (4.5:1) against the background. The label on a primary or destructive button clears AA against that fill. `tests` in `src/theme.rs` lock this.

Borders are mixed, not picked by hand:

| Constant | Value | Themes |
| --- | --- | --- |
| `BORDER_MIX_DARK` | 0.22 | Dark, dusk |
| `BORDER_MIX_LIGHT` | 0.20 | Light, paper |
| `BORDER_STRONG_DARK` | 0.36 | Dark, dusk |
| `BORDER_STRONG_LIGHT` | 0.34 | Light, paper |

`mix` is `foreground * t + background * (1 - t)`, each channel rounded half away from zero. The CSS file stores the result. Don't invent a third border color.

Danger is `#f7768e` on dark themes, `#a61b1b` on paper, `#b42318` on the plain light theme. Warning is `#e0af68`, `#7a5200`, and `#9a6700` the same way.

### Website palette

Gold `#f0a12a` on near-black `#090909`. Text `#ecebe8`, secondary `#b8b6b1`, muted `#9a9893`. Use `--bg`, `--text`, `--line`, `--panel`, `--accent`. Don't point marketing chrome at `--app-*`.

The updates page follows the system. In light mode the same roles sit on warm paper (`#f6f5f2`), and the gold is darkened (`#9e660a`, links `#8a5608`) so a label on the button and a link on the page both clear WCAG AA. Those values live in `tokens.css` on `html[data-page="updates"]`. The rest of the marketing site stays on near-black.

### Print and PDF

Preview and PDF are ink on paper, not a screen theme. CSS print colors are `--print-*` in `tokens.css`. The PDF itself writes the operator strings in `src/pdf.rs` (`ANSWER_INK` and the body gray). Those bytes are covered by tests. Don't recolor a PDF to match the screen accent.

## Typography

The editor is monospace (JetBrains Mono, bundled, 16px). UI chrome is the same proportional face the platform uses for egui; the website uses Inter for UI and JetBrains Mono for code and the demo.

| Step | Rust | CSS | Use |
| --- | --- | --- | --- |
| Caption | `TYPE_CAPTION` 12 | `--type-caption` 12px | Section labels, hints, footnotes |
| Body | `TYPE_BODY` 13.5 | `--type-body` 14px | Buttons, descriptions. The web shell rounds to 14 |
| Label | `TYPE_LABEL` 14 | — | Setting titles, sidebar rows |
| Title | `TYPE_TITLE` 16 | `--type-title` 16px | Dialog titles, the page title in a preview |
| Display | `TYPE_DISPLAY` 22 | `--type-display` 22px | The wordmark in About |

Weights: regular for body, medium is not a separate file on desktop. Don't bold a whole sentence to make it important. Use the title size, or the accent, once.

The editor size (`FONT_SIZE`, 16) and the answer column geometry stay put. A design change does not reflow a page someone has open.

## Spacing

A 4px grid.

| Token | px | Typical use |
| --- | --- | --- |
| `SPACE_1` | 4 | Inside a tight row |
| `SPACE_2` | 8 | Between a title and its description |
| `SPACE_3` | 12 | Between stacked controls |
| `SPACE_4` | 16 | Card padding, dialog inset on the short side |
| `SPACE_5` | 24 | Page margin in the editor |
| `SPACE_6` | 32 | The gap before an inline answer (`GAP`) |

`HIT` is 32. A button, a sidebar row, a switch's hit target, and a menu row are at least this tall. The sidebar's top bar stays 28 (`BAR_ROW`) so it lines up with the ⋯ above the page. Don't change `BAR_ROW`, `SIDEBAR_ROW` (32), or `ICON_BOX` (24) to "match the grid" — tests pin those centers.

## Radii

| Rust | px | CSS | Use |
| --- | --- | --- | --- |
| `RADIUS_SM` | 6 | `--radius-xs` | Small icon buttons |
| `RADIUS_MD` | 8 | `--radius-sm` | Buttons, fields, menu rows, popups |
| `RADIUS_LG` | 12 | `--radius-md` | Dialogs, cards, toasts |
| — | 16 | `--radius-lg` / `--radius` | Marketing cards only |

Don't introduce 4, 10, or 14. A pill (`--radius-pill`) is for a marketing chip, not an app control.

## Elevation

One shadow: offset `(0, 4)`, blur 18, no spread. Black at 80 alpha on dark themes and 28 on light themes (`design::shadow`). Dialogs, popups, and toasts use it. The editor, sidebar rows, and answer text do not.

The website maps the same idea to `--shadow-sm`, `--shadow-md`, and `--shadow-lg`, all mixed from `--black`.

A modal scrim is black at 160 alpha on dark themes and 96 on light (`design::backdrop`). One scrim for every dialog.

## Motion

| Token | Value | Use |
| --- | --- | --- |
| `--dur-fast` | 120ms | Color and background on the web shell |
| `--dur` | 180ms | A panel the website opens |
| `--ease` | `cubic-bezier(0.2, 0, 0, 1)` | Both |

Desktop paints immediately. Don't add a tween to egui.

Don't animate:

- The editor or the answer column
- A drag image
- Focus moving between fields
- Anything when `prefers-reduced-motion` is set (the web shell already cuts transitions there)

## Iconography

Icons are drawn strokes, 1.5px, inside a 24px box (`ICON_BOX`). Plus, sidebar, gear, folder, and the ⋯ share that stroke so they match in any font. Don't drop in emoji as chrome, and don't mix filled and stroked icons in one row.

A sidebar row's accent is a 3×16 bar on the open page. Leave that size alone.

## Components

### Buttons

Primary: accent fill, `on_accent()` label, height 32, radius 8, no border (`design::primary`). One primary button in a dialog.

Quiet: background fill, text color, 1px border, same height and radius (`design::quiet`). Cancel, "What's new", "Choose folder…".

Danger: `danger()` fill, `ink_on(danger)` label (`design::danger`). Delete, and delete forever. Not for "Replace" — that confirms an overwrite but stays a primary button.

Do: a verb, sentence case, a hover and a pressed state from `design::apply`.
Don't: a second accent, a button under 32px, or a primary button that is only an outline.

### Inputs and rename fields

A resting field uses the border token. Focus is an accent ring (`design::field`), 1.5px, radius 8. The rename field and the handle field are single line, monospace, 16px.

The placeholder is an example or a short noun ("New name", "handle"), not instructions.

Do: put the error under the field, in `danger()`, at caption size.
Don't: turn the whole field red, and don't cover the word being renamed. The field sits just below the token.

### Toggles and selects

A setting row is a label and a one-line description on the left, the control on the right. The switch is 36×22 inside a 44×32 hit target (`design::toggle`). On is the accent; off is the hover fill with a border. The knob is `on_accent()` when on and the text color when off.

The theme select is a combo box, 148px wide, listing Auto, Light, Dark, Paper, Dusk, Omarchy, Custom. Don't replace it with a grid of swatches.

The stored values do not change meaning. "Hide the answers column" on means the column is hidden.

### Sidebar rows and folders

A row is 32px, radius 8. The open page uses `selection` and the accent bar. Hover uses `hover`. Names that don't fit end in an ellipsis on one line. Folders indent and show a chevron. Dragging draws a border in the accent around the target folder, and a small surface chip under the pointer.

Do: keep the ⋯ column lined up with the buttons in the top bar.
Don't: wrap a name onto a second line, or paint the row border in the selection color.

### Tabs

Settings and About are tabs in one window, beside Keyboard, Syntax, and Tips. The rail row is 32px, radius 8, selection fill when active. A dot on About means an update is available. The hairline between the rail and the page is `border()`, not `selection`.

### Popups and tooltips

One popup at a time. The autocomplete popup, a hover value, and a context menu do not stack. A hover card sits above the token it explains, and flips below if it would leave the window (`hover_tip_rect`). It never covers the token.

Style is `design::popup`: surface, border, radius 8, the shared shadow. Rows aim at 32px. The selected row is `selection`; hover on another row is `hover`.

Don't change the anchor math to restyle a popup. The editor measures the anchor. A screenshot fixture may pin one after that measurement, behind `OMACUS_SCREENSHOT_HELP`.

### Context menus

egui's menu, with the visuals from `design::apply`: radius 8, border, surface fill. Items are sentence case and stay in one line. Shortcuts sit on the right. Destructive items are last, after a separator.

### Dialogs

`design::dialog`: surface, border, radius 12, padding 20×16, the shared shadow, the shared scrim. Titles are 16px. Body is secondary text. Actions are a row: the primary or danger button, then quiet Cancel.

The page switcher and folder rename use the same frame with tighter padding. They are still dialogs.

Do: one question, two actions.
Don't: a third button that repeats Cancel, or a dialog wider than the sentence needs. 380px is enough for replace and handle. 340px is enough for delete.

### Toasts

A surface card, border, radius 12, centered at the bottom. It fades. Errors use `danger()`. Undo and "Open folder" are quiet text buttons in the accent. One toast. A new one replaces the old one.

### Banners

The web install banner, the shared-page banner, the rates note, and the update banner use `--app-accent-soft` (the accent at 14%) and the border token. The update banner says "Update ready" and its button is Reload. It stays up until the open page has been saved. One banner does not cover the first line of the page. Dismiss is a quiet control. The rates note says when cached exchange rates could not be refreshed. A short note in the sidebar says when this browser refused to keep saved pages, and points at Export.

### Empty states

An empty page shows `empty_sheet_hint`: where pages are, and Ctrl+B / Ctrl+J. Secondary text, wrapped, left aligned with the editor. Don't add an illustration, and don't say "sheet".

### Settings

Grouped cards on the surface color, with a caption above each group: Appearance, Answers, Lock, Export, Updates. Each row is a label, a description, and a control aligned to the right. The path of `settings.toml` is a caption under the groups.

Lock is the "Lock again after" combo (1, 5, 15, 30, or 60 minutes, or Never). The web select is `data-setting=lock-minutes`, with 5 selected. Desktop keeps the folder picker, Omarchy, Custom, and update checks. The web app's Export group is a note that the desktop app chooses the PDF folder. The web checkbox `data-setting=hide-answers` still means hide when checked. The lock dialog uses the shared dialog frame. "Remove lock" is a danger button. Unlock, Lock, and Change stay primary.

### About

The wordmark, "Math, the way you write it.", then two cards: the version (and the update actions) and "Private by design". Links are accent, frame-less: Website, Docs, What's new, Privacy, Report an issue, Source code. The license line is muted.

Don't add metrics, testimonials, or a changelog inside About. "What's new" opens the updates page.

### Updates

`omacus.com/updates` is a timeline of releases, newest first. A version is a pill (`--radius-pill`) in the accent. The date is caption size. The headline is the first line of the notes. Small fixes fold into a disclosure under the newer entry. Screenshots sit in the notes, with `--radius` and `--shadow-md`.

Do: use the marketing tokens (`--bg`, `--text`, `--line`, `--accent`). Follow the 4px spacing scale. Keep one accent.
Don't: point the page at `--app-*`, or put a hex color in `site.css`. Light mode only reassigns tokens in `tokens.css`.

### Preview and PDF typography

The preview is a document: title at 16px, body in the proportional face, answers in brackets in the accent (or `--print-accent` / `ANSWER_INK` when printed). Edit is a primary button. Print and Export PDF are quiet buttons. "Delete forever" on a trashed page is danger text, not a filled button, so Restore stays the one primary action.

Don't restyle the PDF operators to follow the screen theme. A printed page is black, gray, and a dark blue that still reads when the printer is grayscale.

## Accessibility

- Text that someone must read meets WCAG AA against its background: 4.5:1 at body size. The theme tests lock the built-ins. A new theme file should be checked the same way before it ships as a default.
- Focus is an accent ring, 1.5px, from `design::apply` (`widgets.active.bg_stroke`) and from `design::field`. Don't remove it, and don't rely on color alone for the open page — the accent bar is there too.
- Every action that a pointer can do has a key. Esc closes the frontmost dialog or popup. The editor keeps its own keys (Ctrl+J, Ctrl+B, Ctrl+\, Ctrl+,).
- Hit targets are at least 32×32, except the sidebar's top bar, which stays 28 so its buttons share a center line with the rows.
- Don't convey state with color alone. A switch moves its knob. A selected row is filled. An error has words.
- Honor `prefers-reduced-motion` on the web. The desktop app has nothing to reduce.

## When you add a component

1. Read this guide.
2. Use `theme` and `design` on desktop, or the custom properties on the web. No new hex.
3. Add a short do / don't under Components, in the same change.
4. Attach before and after screenshots to the pull request. Light and dark, if both change. Desktop and web, if both change. Use made-up pages (Trip Budget, Groceries, `@trip.total`). Never a real customer's names or figures.

`OMACUS_SCREENSHOT=out.png` saves the window after a few frames. `OMACUS_SCREENSHOT_HELP` opens one of `settings`, `about`, `replace`, `switcher`, `preview`, `rename`, `renamed`, `update`, `picker`, or `hover`.
