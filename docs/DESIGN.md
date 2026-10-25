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

`site/app/app.js` does not set color literals. It sets `data-app-theme`, and reads `--app-bg` for the theme-color when a theme is chosen, so the status bar is the same solid colour as the page. Auto keeps the light and dark `theme-color` tags. Built-in palettes in `tokens.css` are checked against `src/theme.rs` by `tests/check_design.py` (borders included). A hardcoded `Color32::from_rgb(…)` or `hex("#…")` outside `src/theme.rs`, or a hex color in any other file under `site/**/*.css`, fails CI. A justified exception is a `design-allow` comment on that line, or a path in `tests/design-allow.txt`.

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

### Release notes

The updates page, the Atom feed, GitHub releases, and What's new in the app use the same words. Write them for someone using Omacus.

- Benefit first. A short title, then one or two sentences. "Lock pages and folders with a password." "A cleaner, calmer look."
- Plain words. No pull request numbers, no commit subjects, no CI, workflow, or tooling names.
- A release that only changes how updates ship says "Behind-the-scenes improvements to how updates ship", and is folded under the neighboring entry.
- A Details link to the full change list is enough for the history.
- Examples use made-up pages: Trip Budget, Groceries, `@trip.total`. Never a real person's names or figures.

## Color

Roles, not one-off hex values. Each built-in theme defines the role. Custom and Omarchy themes derive the same roles from the file they load: `text_secondary` and `text_muted` mix toward the background, `border` uses the mix constants below.

| Role | Rust | CSS | Use |
| --- | --- | --- | --- |
| Background | `background` | `--app-bg` | The page, the top bar, and the safe areas |
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

Preview and PDF are ink on paper, not a screen theme. CSS print colors are `--print-*` in `tokens.css`. The PDF itself writes the operator strings in `src/pdf.rs` (`ANSWER_INK`, the body gray, and the code fill `0.96 0.96 0.95` with stroke `0.75 0.75 0.73`). Those bytes are covered by tests. Don't recolor a PDF to match the screen accent.

A page has a 40pt margin (0.55in, about 14mm) on every side. The footer rule and the page number sit inside that bottom margin, under the text. A fenced code block is Courier, the same width as the body, with 6pt of padding on the sides and 4pt above and below. Its fill is `--print-code` on the web and that gray in the PDF, and its edge is a hairline. A line longer than the measure wraps, including one long token. The web print stylesheet uses the same 0.55in page margin and clears the on-screen preview padding, so the two do not stack.

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

Paragraphs, lists, and the other blocks of body copy use `--prose-gap` (12px, about 0.7em of the 16.5px body, in the same range as `SPACE_3`). It is a length rather than `em`, so a heading that uses it does not widen the gap. A section heading keeps a larger step (`SPACE_5`, `SPACE_6`, or two of `SPACE_6`) so the break stays obvious. Text inside a card, and the lines of a list, stay tighter than `--prose-gap`.

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

### App bar

The bar is a solid surface in the page background (`--app-bg`), the same colour as `theme-color` and the safe areas, with a strong border under it (`--app-line-strong`). It is fixed to the top edge and sits above the page and the drawer scrim. It pads by the top safe area, so the notch and the clock sit on that surface with no seam. The Home Screen status bar is `black-translucent`, which lets the page draw behind it; the bar paints an opaque colour over that whole strip. It is not blurred. The page name, the pages button, and the ⋯ button use the interface face and the text color. Buttons in the bar use the same fill as the bar. The slash before the name stays secondary. The bar has no Private lock. That note lives in About.

On an iPhone where Omacus is on the Home Screen, the document is `100vh` and the shell is pinned to the top and bottom of the viewport, with `viewport-fit=cover`. A percentage height drops that edge-to-edge layout, and the dynamic viewport unit is wrong until the phone is rotated. A shell whose height is `100vh` and whose bottom is left unset stops short and leaves a band. The sidebar, its divider, the page, and the drawer scrim run to the screen edges. Safe areas pad the words, the buttons, and the version. They do not inset the boxes. The safe areas are measured again on launch, after a short wait, and when the window, the rotation, or the visual viewport changes. `html` and `body` use the page background. There is no separate canvas; the page is the shell.

A collapsed sidebar is a rail. Its column is the rail plus the left safe area, so the buttons sit clear of the notch and stay above the page.

Do: keep the bar opaque. No blur and no translucent fill. Keep the light and dark `theme-color` tags in step with `--app-bg`.
Don't: set the page name in the monospace face, paint the bar in `--app-side`, drop the border, or drop `black-translucent` (that insets the web view and leaves a band). Don't size the Home Screen shell with `100%` or `100dvh`, and don't take its height from the window at launch. The border is what separates the bar from the page. Don't put a Private lock in the bar.

### Sidebar footer

A labeled Settings row, About, and the version (`omacus vX.Y.Z`, a link to the updates page). The same footer is used on a phone and in a wide window. It sits above the home indicator. Where pages are kept is said in About, not under the version.

On a phone the pages list is a drawer. Its background runs to the top and bottom of the screen. A scrim covers the rest of the page, below the bar, and reaches the bottom edge. It does not cover the bar. The empty-page hint does not show through it. A closed drawer casts no shadow on the page. The shadow is there only while the drawer is open. A tap on the page beside the drawer closes it. That tap does not select text, move the caret, or open the keyboard. The page menu and a confirm question sit above that drawer, under the bar.

Do: say Settings in words, next to the gear.
Don't: a gear with no label, a version that is not the release in `Cargo.toml`, or a privacy paragraph in the footer.

### Page menu

The ⋯ menu is for the open page: Rename, Share link, Copy / Export, Handle, Lock or Unlock, Remove lock, Move to folder, and Delete. Copy / Export opens a second list: Copy with answers, Copy as Markdown table, and Preview. On the web that list also has Open in the desktop app. New page, New folder, Import, Settings, About, and the desktop download live in the sidebar or in Settings. A backup of every page lives in Settings, as Export everything.

Do: a row at least 32px. Arrow keys move through the list. Esc closes it, or steps back from Copy / Export. Back is a row, so a thumb can leave the second list.
Don't: put Settings, New page, or the storage note in the page menu.

### Sidebar rows and folders

A row is 32px, radius 8. The open page uses `selection` and the accent bar. Hover uses `hover`. Names that don't fit end in an ellipsis on one line. Folders indent and show a chevron. With section captions, the caption is the outer level. Pages under it sit one step in, and pages inside a folder sit one step further. A chevron lines up with the chevrons on that level, and a page name lines up with a folder name. Without captions, a page and a folder on the same level share that name column. Dragging draws a border in the accent around the target folder, and a small surface chip under the pointer.

The ⋯ on a row opens that row's menu in the top layer, above the sidebar and the page, beside the button. The whole menu stays on the screen.

A fresh library opens on Welcome to Omacus. That page is the first row, above the Examples folder.

Do: keep the ⋯ column lined up with the buttons in the top bar.
Don't: wrap a name onto a second line, or paint the row border in the selection color. Don't leave the row menu inside the page, where the sidebar can cover it.

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

Pick one line uses the same frame at 380px. It shows the two copies of one line and two actions: Keep this line, then Use the other line. A locked page asks Pick one copy and does not show the locked text.

On a phone, Settings and About are one full-width sheet that starts at the bottom of the top bar. The title and Close stay on screen, and the sheet clears the home indicator. A wide window keeps the centered card.

### Toasts

A surface card, border, radius 12, centered at the bottom. It fades. Errors use `danger()`. Undo and "Open folder" are quiet text buttons in the accent. One toast. A new one replaces the old one.

### Banners

The web install banner, the shared-page banner, the rates note, and the update banner use `--app-accent-soft` (the accent at 14%) and the border token. The update banner says "Update ready" and its button is Reload. It stays up until the open page has been saved. One banner does not cover the first line of the page. Dismiss is a quiet control. The rates note says when cached exchange rates could not be refreshed. A short note in About says when this browser refused to keep saved pages, and points at Export.

### Saved pages when the app cannot start

If the web app cannot start, the loading line is replaced by the pages already in this browser. The note says the pages are still in this browser. Each page is a full-width row on the sidebar surface, with the border and the small radius. The open page is plain text under the list. Export everything saves a copy. A locked page is named Locked page and its words stay hidden. The panel uses the page background and the secondary text color. It only reads the saved pages.

Do: keep the page list and the export button on screen until the app itself is up.
Don't: clear the saved pages, or hide the list behind another spinner.

### Editor

The page fills the sheet. Focusing it shows the caret, not a ring, a shadow, or a tap highlight. Otherwise the contenteditable's focus outline, including the one from the site stylesheet, becomes a rounded frame around the whole page on iPhone Safari. A screen-reader announcement stays in the page and is clipped so it is not drawn on the sheet.

Do: keep the accent focus ring on buttons, menus, sidebar rows, and fields. Keep `img.cm-widgetBuffer` inline, so a short line holds its answer beside the text.
Don't: outline the editor, or restyle the empty-page hint. It stays secondary text.

### Empty states

An empty page shows `empty_sheet_hint`: where pages are, and Ctrl+B / Ctrl+J. Secondary text, wrapped, left aligned with the editor. Don't add an illustration, and don't say "sheet".

### Settings

Grouped cards on the surface color, with a caption above each group: Appearance, Answers, Lock, Export, Updates. The web app adds a This copy group after Export. Each row is a label, a description, and a control aligned to the right. The path of `settings.toml` is a caption under the groups.

Lock is the "Lock again after" combo (1, 5, 15, 30, or 60 minutes, or Never). The web select is `data-setting=lock-minutes`, with 5 selected. Desktop keeps the folder picker, Omarchy, Custom, and update checks. Export everything is a quiet button in the Export group, on the desktop and on the web. It writes one file of pages, folders, settings, and Recently deleted. Locked pages stay locked in that file. The web app's Export group also holds Import pages and the desktop download, plus a note that the desktop app chooses the PDF folder. The web checkbox `data-setting=hide-answers` still means hide when checked. The lock dialog uses the shared dialog frame. Choosing Lock on a page with no password asks for a password and a confirmation, tells you to save it in a password manager, then locks the page. Remove lock asks for the current password and stores the page without one. A wrong password leaves the page as it was. "Remove lock" is a danger button. Unlock, Lock, and Change stay primary.

This copy is web only. Its caption is Build. The hash sits under the help line in `.build-hash`: mono, caption size, `--app-fg`, wrapping anywhere. An unpublished copy says Not a published build. The node is `data-build-hash`.

### About

The wordmark, "Math, the way you write it.", then two cards on the desktop: the version (and the update actions) and "Private by design". That card says pages stay on this computer as plain text files. In the web app, About says pages stay in this browser, on this device, and with sync on a sealed copy can go to other phones and computers. A share link carries the page after the #, export a copy from Settings, and on iPhone and iPad add Omacus to the Home Screen so Safari keeps them. If the browser refused to keep saved pages, a line there says so and points at Export. Links are accent, frame-less: Website, Docs, What's new, Privacy, Report an issue, Source code. The license line is muted.

Don't add metrics, testimonials, or a changelog inside About. "What's new" opens the updates page.

### Updates

`omacus.com/updates` is a timeline of releases, newest first. A version is a pill (`--radius-pill`) in the accent. The date is caption size. The headline is the release's title, a bold line at the top of the notes, and the sentences under it are the body. A release marked minor or internal folds into a disclosure under the newer entry. Screenshots sit in the notes, with `--radius` and `--shadow-md`. A Details link points at the full change list.

Do: use the marketing tokens (`--bg`, `--text`, `--line`, `--accent`). Follow the 4px spacing scale. Body copy under a release uses `--prose-gap`. Keep one accent.
Don't: point the page at `--app-*`, or put a hex color in `site.css`. Light mode only reassigns tokens in `tokens.css`.

### Story

`omacus.com/story` is a first-person account of why Omacus exists and how it is built. The counts and the timeline are filled in when the site is built (`site/tools/story.py`), from the repository and the release list. A figure that cannot be counted is left off. Sync is in the published web app. Turn on sync is in the sidebar.

The page uses the same marketing frame as the updates page: a kicker, a serif word in the title, version pills, and the timeline. It stays on the near-black marketing palette.

Do: use the marketing tokens. Follow the 4px spacing scale and the radius scale. Paragraphs use `--prose-gap`; a section heading stays on the larger steps. Keep one accent. Write in the first person, plain and short.
Don't: hardcode a count or a date in the page, point the page at `--app-*`, or put a hex color in `site.css`.

### Sync page

`omacus.com/sync` and the band on the homepage. The page explains how sync works now: pages are encrypted on the device, the server cannot read them, 12 recovery words, linking a phone with a code, offline edits, and a nightly sealed copy kept for about 30 days. It says what the server can see (that a sealed copy exists, its size, when it was saved, how many phones are linked, and the sealed bytes) and what it cannot see (page text, page names, recovery words, and keys). A video stays off the page until there is one to play. The Turn on sync button in the sidebar explains sync, then continues in the web app. This version syncs one named set of pages. The page does not offer a form, a sign-up, or a waitlist. The words shown on the page are an example, labeled as one. The page has its own share image, `site/img/og-sync.png`.

Do: use the marketing tokens. One accent. Say the recovery phrase is 12 words. Paragraphs on the sync page and in this band use `--prose-gap`. Don't give them a margin of their own. When a video is added, the slot uses the raised surface, the line, and the medium radius.
Don't: collect an email, call the phrase 24 words, or say sync is coming soon. Don't show an empty video slot, and don't embed a third-party player until there is a video. The launch video may show sync as a feature.

### Sync test

A private test, labeled `1.0.0-beta.40`. A separate app named omacus Test, on Mac and on Linux, opens this same test. It is not the desktop app. The desktop app shows Turn on sync in the sidebar. That explanation continues in the web app, and the desktop window does not seal pages. `relay/www` is the small test page. The same screens also live in the web app's Settings, on a Sync tab, including the published app. When sync has never been set up, Turn on sync is in the sidebar, and on a wide window it is also in the top bar. It opens a short explanation: pages are encrypted before they leave, the server cannot read them, 12 recovery words, and linking a phone. Set up sync then opens the existing first screen. A narrow phone keeps those words in the pages list and leaves the top bar for the page title. The top bar does not say Sync paused before setup. `sync-test.omacus.com`, or `data-sync-test="1"` on the page, keeps that copy's pages separate. One question or one instruction on each screen. One primary button. A Back, Cancel, or No button is on every screen that is not the home screen. The typed code is the main way to join. Scanning is a shortcut. Each step says what to do on the other phone or computer. Starting a new set of pages asks for an invite code. Joining, and opening pages with the recovery words, do not. The 12 words are called recovery words everywhere on these screens.

Before, the first screen offered three peer actions, and later screens talked about "devices" without saying which phone was doing what.

- **Sync.** "Pages can open on your other devices. They stay on your devices. This test cannot read them." Turn on sync. Add this device. Use your recovery key.
- **Turn on sync.** Step 1 of 3. "Next you will get 12 words." Turn on sync. Not now.
- **Save your recovery key.** Step 2 of 3. Twelve words and a picture. "Write them down, or print the sheet." Copy. Print. I saved them.
- **Pick the three words.** Step 3 of 3. "From your sheet, pick word 3, word 7, and word 11." Confirm. Back.
- **Sync is on.** "This device" and "Linked device", each with Remove. Add a device. Back. No success screen, and no way to turn sync off.
- **Add a device.** A picture first, then a short code. "On the new device, scan this code or type it."
- **Add this device.** Scan code. Type code. Back. A miss could say the server had no pages.
- **Type the code.** A field. "It looks like K7QM-4R2P." Continue. Back.
- **Scan code.** "Point the camera at the code." Type code instead.
- **Add this device, waiting.** The six numbers. "Tap Approve." Cancel.
- **Approve a new device?** "New device wants to open your pages." Approve. Not now.
- **Use your recovery key.** One box. Scan recovery sheet. Open my pages.

After, the words name a phone or computer, and each screen is one step.

- **How sync works.** "Pages are encrypted on this phone before they leave. The server keeps them, and it cannot read them." "You get 12 recovery words." "You link another phone or computer with a code." Set up sync. Not now. Set up sync opens the existing first screen. Not now closes Settings. This screen is what Turn on sync opens. The Sync tab still opens on the first setup screen.
- **Turn on sync.** "Your pages can open on your other phones and computers, and they stay there." "Start fresh if this is the first time. Join if your pages are already on another phone or computer." A name field is filled in with a guess. omacus Test fills in omacus Test on Linux, or omacus Test on Mac, when the browser string says omacus-test. A browser fills in its own name, such as Chrome on Linux, Safari on Mac, Firefox on Linux, Edge on Windows, or iPhone. The desktop app fills in Omacus on Linux or Omacus on Mac when the browser string says omacus-desktop. The person can change the name. Start fresh. Join my existing pages. Back.
- **Have an invite code?** "Type the code you were given. This starts a new set of pages here. It is not the code on your other phone." The field. Start fresh. Back. A wrong or used code says "That invite code didn't work. Check it and try again." This screen is only on Start fresh.
- **Save your recovery words.** "Print them or copy them. They are the only way back in if you lose every phone and computer. You will not see them again." The words, then "The same words" on the picture. Print. Copy. I saved them. Back. Back does not turn sync on. Print is the primary button. This check happens once, at the start.
- **Do these match your paper?** "Pick word 3, word 7, and word 11." Six choices each, nothing to type. These match. Back to the words. A miss says "That's not word 7. Look at your paper and try again." These match opens the home screen. There is no extra done screen.
- **Sync is on, home.** "Your pages stay on your phones and computers." The name for these pages starts as My pages, and it can be changed here. The name of this phone or computer is the guess from the first screen. This screen has no separate name field for it. Each linked phone or computer is a row with Rename. Any other row also has Remove. This phone, or this computer, keeps a small "This phone" or "This computer" label, and its row uses the accent. Rename on this row works the same way as on the others, and the new name shows on every linked phone and computer. The name typed before Join is the name on that row. Until a name arrives, that row says "Phone or computer". "Remove stops that phone or computer from getting new changes. Pages already there stay there. Rename changes the name on every phone and computer." Add a phone or computer. Turn sync off. Back. This phone does not offer Remove for itself. The list shows every linked phone and computer, including this one, even when two names match. On a phone, the list scrolls and those three buttons sit under it.
- **Remove Harbor phone?** The heading uses that phone's name. When the name is still Phone or computer, the heading is Remove this phone or computer? "It will stop getting your pages. Pages already on it stay there." Remove. Keep it.
- **This phone was removed from sync.** "Your pages are still here." Join again. Back. The top bar says Sync paused. There is no device list, no Remove, and no extra step to turn sync off. On a computer the heading says "This computer was removed from sync."
- **An approval that cannot be opened.** The screen names the check. "The approval's list didn't check out." "The saved copy's list didn't check out." "The approval's list version doesn't line up." "The saved copy's list version doesn't line up." A phone that is already on the list says so. A phone that is no longer on the list says so.
- **Add a phone or computer.** "On the other phone or computer, open Sync and tap Join my existing pages. Type this code. It works for about 5 minutes, and only once." The code `K7QM-4R2P` comes first. "Or scan this picture." Back.
- **Join my existing pages.** "On a phone or computer that already has your pages, open Sync and tap Add a phone or computer. Type the code you see there." The name field is on this screen, filled in with the same kind of guess, and it can be changed before Join. The code field is on this screen. If this phone already has pages: "Pages already on this phone will be combined with those. Nothing is deleted." Join. Scan the code instead. Use my recovery words instead. Back. A bad code says "That code didn't work. Check it on the other phone or computer." The join path never says there are no pages.
- **Scan the code.** "Point the camera at the picture on your other phone or computer." Type the code instead.
- **Approve on your other device.** "The same six numbers should be on that phone or computer. Tap Yes there only if they match." The numbers are grouped as `482 916`. Waiting for that phone or computer. Cancel. The six numbers stay. Typing the invite code is not the same as looking at those numbers.
- **Do these match?** "The same six numbers and the same four words should be on your other phone or computer." The six numbers, then the four words. These match. Cancel. Yes stores this phone on the list before the pages open. The pages open only after These match. If this phone stops waiting, the same code works again. If it never finishes after Yes, that row is removed and the same code works again. A code that was already used, or that has expired, still does not work.
- **Is this Harbor phone?** The heading names the phone or computer that asked. With no name yet it stays "Is this your phone or computer?" "It wants to open your pages. The same six numbers and the same four words should be on that screen. Those numbers only check this step. Tap Yes, approve it only if they match." The six numbers, then the four words. Yes, approve it. No. Yes returns to the home screen. A phone that stops waiting drops off this screen on its own, and Yes cannot approve it. The code stays until it expires, and the next phone that uses it is asked about too. While Sync is closed, that request shows as Device waiting on the status.
- **Use your recovery words.** A name for this phone or computer, then "Type or paste the 12 recovery words. Numbers and capitals are fine." The same combine line, when this phone already has pages. Open my pages. Scan the paper instead. Back. Open my pages adds this phone or computer to the list and goes to the home screen. That row is marked, and it has no Remove. The other phones and computers see it once that copy is saved. There is no second word check. Typing the 12 words is the check. A problem on this screen sits in a box under the title. A computer that is not on the list does not say sync is on.
- **Turn sync off on this phone?** "Your pages stay here. This phone will stop sharing them. The recovery words still open them." Turn sync off. Not now. Turn sync off returns to the first screen. The top bar says Sync paused.

Closing Settings, leaving the Sync tab, or pressing Esc keeps sync running. Edits keep arriving. Removing a phone or computer stops it from getting later pages, and the recovery words still open those later pages. When the words are right but they cannot open the newest copy, this phone opens the newest copy they can, combines it with the pages already here, and nothing is deleted. The home screen then says, in a box under the title: "Some changes made after a phone or computer was removed may be missing. They were locked to phones and computers that are no longer linked. Pages already here were combined with the ones these words could open. Nothing was deleted." Sync seals a fresh copy for these words and for this phone, so the next recovery opens the latest pages. A phone that was still linked can open that copy later and combine what it still has.

The home screen is one named set of pages, plus the pages that stay on this phone or computer. The name of the set and the names of the phones live inside the sealed pages. The sidebar uses that name as a caption, with the count, then the pages in the set. A second caption, On this phone or On this computer, lists the pages that stay here. The caption, then the pages, then the pages in a folder, each sit one step to the right. Chevrons on one level line up, and a page name lines up with a folder name. Each caption collapses. A page moves by dragging it from one caption to the other. Ctrl-click or Shift-click selects many, and the page menu has Select and Move to… for a touch screen or the keyboard. Move to… lists the set and On this phone or On this computer, so another set can be added on this same screen later (planned for v1.1). This version has one set. This phone keeps a list of sets. Each entry has its own device file, revision, last synced pages, paused flag, and name. This version opens one entry. The account id and the sealed marker stay as they are. Moving asks first. Out of the set: the page stays on this phone, other phones take it out of the set, the copy already there stays and will not get later changes, and nothing is deleted. Into the set: other phones get this copy, and a page they already have stays one page. Undo puts the pages back. Moving keeps the same page.

When sync has never been set up, the top bar does not say Sync paused. Turn on sync is the entry, in the sidebar and, on a wide window, in the top bar. After sync is on, the status sizes to its words. A cloud shows the state: a check when these pages are synced, the cloud alone while a save is still going, a slash when this phone is offline, and a pause after sync is turned off or this phone or computer is removed. Not saved is the error, in the danger color, and tapping the status opens Sync. When a phone or computer is waiting to join and Sync is closed, the status says Device waiting, in the top bar and in the sidebar. That word stays readable on a narrow phone. Tapping it opens Sync on the approve screen. It goes away when that phone is approved, or when the request expires or goes stale. The same cue is there for a wide window, a phone, and the desktop app's browser string. The desktop window itself still does not seal pages, so the cue lives in the web app, which is where setup continues. Synced stays while sync is quiet. Syncing replaces it only after about a second of work that is still waiting, then Synced comes back, and the two do not take turns. Offline means the edits are saved here. On a narrow phone the word hides and the cloud remains, with the full status as the button's name. A deleted line stays deleted. A line typed on the other phone at the same time stays. A save uploads the pages that changed. A line typed while that save is still going out is sent too. Only one open tab asks to approve a phone or computer, or to remove one. A request with no signature, a repeated request, or a request from a removed phone is refused. An older sealed copy is refused, and the phone says "That copy is older than the one already on this phone." The sidebar version line reads `omacus 1.0.0-beta.40`. On a narrow phone the version label in the top bar is hidden, so the page title keeps the room. The closed page list casts no shadow and leaves no line along the left edge of the page. When On this computer has no pages, a line under that caption says they stay on this computer. Rename shows the word Name, then Save and Cancel. The approval screen names the phone or computer that asked. The six numbers stay on the approve screen and the join screen, where both sides compare them, and they are not repeated on each row. Remove uses that phone's name. When the name is still Phone or computer, the heading is Remove this phone or computer? Not saved on the Sync tab includes the server's message. It does not include a code that failed on the phone that is joining.

The admin page lives at `/admin` on the sync host. It mints an invite code, lists the codes, pauses new signups, sets the account, storage, device, and rate limits, and removes a sealed copy after the account id is typed again. It shows the last nightly copy, the status of each account, and any copy that did not save. Restore asks first, then writes one account or every account from a chosen day. Stats show how many accounts and phones or computers are stored, the total size, how many accounts were used in the last day and the last week, how many accounts were created in the last week, and the last nightly copy: when it ran, whether it saved, and its size. Each account shows when it was created, when it was last used, when its sealed copy last changed, its size, how many phones and computers are linked, and how many times it was used in the last day. A time reads as how long ago it was, and hovering shows the exact time. The device count is the phones and computers on that account's list. It does not show page text or names, and it cannot open a page. A missing or expired Access sign-in says "Sign in again." A wrong admin key says "That key didn't work." Create code is the one primary button. Pause, Resume, Save limits, and Copy now are quiet. Remove sealed copy and Restore sealed copy are danger text. Cancel on the restore confirm is quiet. The code is shown once.

The test copy keeps its own pages, separate from the published app. Offline, the pages already on the phone stay there. The server still never sees the words or the pages.

Do: use the app tokens. Sentence case. One primary button on each screen. Trip Budget for a made-up page.
Don't: put the test label on the website, hide the Sync tab in the published app, write the words anywhere but the phone, name the parts of a key in the words a person reads, or show page text or names on the admin page.

### Foldable showcase

Under "Nothing to install" the homepage shows one small unfolded phone, about 440px wide and centered, with a one-line caption. It is not its own headline. Unfolded, the device lies flat, landscape, about 4:3. The rim is polished stainless (`--metal-hi` through `--metal-edge` in `tokens.css`), with a thin black bezel, large corner radii, two small buttons on the right edge, and a soft shadow. A status strip sits above the app on both halves: the punch-hole camera and the time live in that strip on the right, clear of the app's own bar. The halves meet at a barely-there crease: one pixel of highlight and one pixel of shadow, each at 8% opacity. The black bezel stops at the outer edges, so the crease is not a bar through the note. The soft shadow belongs to the whole phone; each half does not cast one, or it would fall across the page. Inside, the page list is the slim sidebar and Trip Budget fills the rest, so the note continues across the crease. The picture is the dark app, rendered near 720px wide, with the page list about 30% of that width, so the type stays proportional and the page names fit. A line does not wrap. The marketing page itself stays near-black.

Do: use the metal tokens for the rim and the buttons, and the marketing tokens for the crease, the time, and the caption. The caption uses `--type-caption`, `--faint`, and `--prose-gap`. The picture has width, height, lazy loading, and alt text that names the page and the two panes. Corners and padding stay on the radius and 4px scales.

Don't: give it a kicker or a serif headline, name a phone brand, say "PWA", or point the type at `--app-*`. The screen fill behind the picture may use `--app-bg`, because that fill is the app. Don't reuse the `.shot` crop. Don't put a hex color in `site.css`. Don't ship a second, light screenshot.

### Preview and PDF typography

The preview is a document: title at 16px, body in the proportional face, answers in brackets in the accent (or `--print-accent` / `ANSWER_INK` when printed). Edit is a primary button. Print and Export PDF are quiet buttons. "Delete forever" on a trashed page is danger text, not a filled button, so Restore stays the one primary action.

A fenced code block in the on-screen preview uses the hover fill, the border token, an 8px radius, and the monospace face, with 8px of side padding and 6px above and below. A long line wraps. Print and PDF use the paper fill and Courier, above, not the screen accent.

Don't restyle the PDF operators to follow the screen theme. A printed page is black, gray, and a dark blue that still reads when the printer is grayscale.

## Accessibility

- Text that someone must read meets WCAG AA against its background: 4.5:1 at body size. The theme tests lock the built-ins. A new theme file should be checked the same way before it ships as a default.
- Focus on a control is an accent ring, 1.5px, from `design::apply` (`widgets.active.bg_stroke`) and from `design::field`. Buttons, menus, sidebar rows, and fields keep it. The editor surface does not: the caret is the indicator, and a ring there frames the whole page. Don't remove the ring from a control, and don't rely on color alone for the open page — the accent bar is there too.
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
