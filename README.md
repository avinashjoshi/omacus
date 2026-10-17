# Omacus

*Oh-mah-kus.* A notepad calculator for Omarchy, Linux, macOS and Windows.
**[omacus.com](https://omacus.com)** · [Docs](https://omacus.com/docs/) · [Download](https://omacus.com/#download)

[![Watch Omacus in one minute](assets/video-thumb.jpg)](https://omacus.com/#video)

Type on the left, results appear on the right.

```
Rent: $1,450                         $1,450
Groceries $320 + $85                   $405
Internet €40 in USD                  $44.86
total                             $1,899.86

rate = $85/hour                 $85 / hour
rate * 6 hours * 4                   $2,040
prev in EUR                   1,818.97 EUR
```

## What it understands

| | |
|---|---|
| Arithmetic | `2^10`, `sqrt(16)`, `10 mod 3`, `5 x 3`, `10 times 3`, `1.5k` |
| Percentages | `20% of 340`, `5% on 200`, `15% off $1,299`, `$120 as % of $480`, `5% of what is 6`, `18% tip on $62` |
| Splitting | `$84 split 3 ways`, `split $240 between 4 people` |
| Units | `3.5 km in miles`, `2 hours + 45 min in minutes`, `72 fahrenheit in celsius`, `10 GB / 4` |
| Downloads | `4 GB at 100 Mbps` → `5 min 20 s`, `700 MB @ 10 MB/s` |
| Currency | `$30 in EUR`, `€50 + £20 in USD`, `£250 in USD` (live rates, cached 12h) |
| Variables | `rent = $1200`, then `rent * 12` |
| Sheet words | `prev`, `sum`/`total`, `avg`/`average` (over the lines since the last blank line or header) |
| Dates | `today`, `now`, `tomorrow + 2 weeks`, `now - 90 minutes` |
| Counting days | `days until Dec 25`, `weeks until Christmas`, `days since Jan 1`, `workdays in November`, `days between Mar 1 and Jun 30` |
| Time zones | `6pm PST in Tokyo`, `time in Sydney`, `now in London`, `10:30 London to New York`, `3pm PST in IST` |
| Bases | `255 in hex`, `0xff in decimal`, `255 in binary` |
| CSS & colours | `16px in rem`, `12pt in px`, `#ff8800 in rgb`, `rgb(51, 102, 153) in hex`, `hsl(150, 60%, 45%)` (with a swatch) |
| Notes | `# Header`, `// comment`, `"quoted text"`, `Label: 40`, stray prose is ignored |

Math, units and currency come from [fend-core](https://github.com/printfn/fend); `src/engine.rs`
adds the notepad conventions on top.

## Using it

- On Omarchy, `SUPER + CTRL + ALT + =` opens Omacus (or focuses it), floating and centered.
- `Esc` or `Ctrl+Q` closes it. Nothing is ever lost: like Sublime Text, every sheet is saved as you
  type and reopens exactly where you left it, cursor included.
- Click a result to copy it; `Ctrl+Shift+C` copies the last result, and `Ctrl+Shift+S` copies
  the whole sheet with its answers lined up (`Ctrl+Shift+M` as a Markdown table).
- `Ctrl` `+` / `-` zooms.
- Colors follow the active Omarchy theme (`colors.toml`) and update live on `omarchy theme set`.

### Sheets

| Key | |
|---|---|
| `Ctrl+N` | New sheet |
| `Ctrl+P` | Go to a sheet by name |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` (or `Ctrl+PgDn` / `PgUp`) | Next / previous sheet |
| `Ctrl+1` … `Ctrl+9` | Jump to sheet |
| `Alt+↑` / `Alt+↓` | Move sheet up / down |
| `Ctrl+Shift+Delete` | Delete sheet (Undo appears; moved to `trash/`, not destroyed) |
| `Ctrl+B` | Toggle the sidebar |

Right-click a sheet in the sidebar for the same actions. A sheet's title is its first line.
Sheets are plain text in `~/.local/share/omacus/sheets/`; order, cursors and the sidebar
state are in `~/.local/share/omacus/state.json`.

### Autocomplete

Suggestions for variables, units, keywords and time zones appear as you type.
`Tab` accepts, `↑`/`↓` choose, `Esc` dismisses. `Enter` always inserts a newline.

CLI:

```sh
omacus '$100 in EUR'
printf 'a = 5\na * 3\n' | omacus -
omacus --version
omacus update        # install the latest version
```

## Install

Omacus runs on **Omarchy**, any **Linux**, **macOS** and **Windows**. Every release on the
[Releases page](https://github.com/avinashjoshi/omacus/releases/latest) has notes and downloads
for each platform.

### Omarchy

One line installs the app with its launcher entry and icon:

```sh
curl -fsSL https://omacus.com/install | bash
```

It then offers to add a keybind and a floating-window rule to your Hyprland config. The suggested
key is `SUPER + CTRL + ALT + =`; pick any other and it's checked against your existing bindings
first, so nothing of Omarchy's gets overridden. If your keyboard has a Calculator key, it can
open Omacus too. Your config files are backed up, nothing is added twice, and changes are rolled
back if Hyprland reports an error.

For unattended installs, pass the answer instead:

```sh
curl -fsSL https://omacus.com/install | bash -s -- --hyprland                 # default key
curl -fsSL https://omacus.com/install | bash -s -- --key "SUPER + ALT + N"    # your key
curl -fsSL https://omacus.com/install | bash -s -- --no-hyprland              # leave Hyprland alone
```

Colors follow your Omarchy theme and switch with it.

### macOS

```sh
curl -fsSL https://omacus.com/install | bash
```

Installs `Omacus.app` into Applications (Apple Silicon and Intel) and opens without a security
prompt. Or download the .dmg for your chip ([omacus.com/download](https://omacus.com/download/#mac)) and drag Omacus to
Applications; the app isn't notarized yet, so the first time, open it once, then go to
**System Settings → Privacy & Security** and click **Open Anyway**
(or run `xattr -dr com.apple.quarantine /Applications/Omacus.app`).

### Windows

1. Download [`omacus-windows-x86_64.zip`](https://github.com/avinashjoshi/omacus/releases/latest/download/omacus-windows-x86_64.zip)
   and unzip it anywhere (for example `%LOCALAPPDATA%\Programs\Omacus`).
2. Run `omacus.exe`. If SmartScreen warns about an unknown publisher, choose
   **More info → Run anyway**.
3. Optional: right-click `omacus.exe` → **Pin to Start**. A pinned shortcut's **Properties** let
   you set a shortcut key.

It works from a terminal too: `omacus.exe "$100 in EUR"`.

### Ubuntu / Debian

```sh
curl -fsSLO https://github.com/avinashjoshi/omacus/releases/latest/download/omacus-linux-x86_64.deb
sudo apt install ./omacus-linux-x86_64.deb
```

### Fedora / openSUSE

```sh
sudo dnf install https://github.com/avinashjoshi/omacus/releases/latest/download/omacus-linux-x86_64.rpm
```

### Arch and any other Linux

Install for your user, no root needed, into `~/.local/bin` with a launcher entry:

```sh
curl -fsSL https://omacus.com/install | bash
```

Or download [`omacus-linux-x86_64.tar.gz`](https://github.com/avinashjoshi/omacus/releases/latest/download/omacus-linux-x86_64.tar.gz),
unpack it and run `./install.sh`. Prebuilt Linux binaries are x86_64; on ARM, build from source.

### From source

Any platform with [Rust](https://rustup.rs):

```sh
git clone https://github.com/avinashjoshi/omacus && cd omacus
cargo build --release          # binary in target/release/
./install.sh                   # Linux: builds and installs for your user
```

On other desktops, bind a global shortcut to `omacus` in your keyboard settings.

### Where things live

| | Linux | macOS | Windows |
|---|---|---|---|
| Sheets | `~/.local/share/omacus` | `~/Library/Application Support/omacus` | `%APPDATA%\omacus` |
| Custom theme | `~/.config/omacus/theme.toml` | `~/Library/Application Support/omacus/theme.toml` | `%APPDATA%\omacus\theme.toml` |
| Settings | `~/.config/omacus/settings.toml` | `~/Library/Application Support/omacus/settings.toml` | `%APPDATA%\omacus\settings.toml` |

Colors: your `theme.toml` if present (Omarchy `colors.toml` format), else the active Omarchy theme,
else the system's light or dark mode. On macOS, shortcuts use ⌘ wherever this README says Ctrl.

### Updating

The version sits at the bottom of the sidebar, with a dot when a new one is out; click it to update
and restart. About (F1) has a Check for updates button, or run `omacus update`. Omacus checks GitHub for the latest version when it starts (at most once an hour) and every
4 hours while open;
`check_for_updates = false` in `settings.toml` turns that off. On Windows it swaps in the new `omacus.exe`
where it is. Installed from a .deb or .rpm, the notice links to the download instead.

## Feedback and issues

Found a wrong answer, a crash, or something Omacus should understand?
[Open an issue](https://github.com/avinashjoshi/omacus/issues/new/choose). For calculations,
paste the exact line you typed and what you expected; that's usually all it takes to fix.
Ideas and feature requests are welcome too, and so are pull requests.

## Releasing

Bump `version` in `Cargo.toml` and push to `main`. The
[Release workflow](.github/workflows/release.yml) notices the new version, runs the tests on
Linux, Windows and macOS, builds every package, then tags `v<version>` and publishes a GitHub
release with notes written from the commits since the last release
([`release-notes.sh`](.github/scripts/release-notes.sh)). Commit body lines starting with `- `
show up as sub-points in the notes.

## License

[MIT](LICENSE) © 2026 Avinash Joshi. The bundled JetBrains Mono font is under the
[SIL Open Font License](assets/fonts/OFL.txt).
