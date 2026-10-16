# Omacus

*OM-a-kus, like abacus.* A notepad calculator for Omarchy / Arch Linux, in the spirit of [Numi](https://numi.app).
Type on the left, results appear on the right.

```
Rent: $1,450                         $1,450
Groceries $320 + $85                   $405
Internet €40 in USD                  $44.86
total                             $1,899.86

rate = $85/hour                 $85 / hour
rate * 6 hours * 4                   $2,040
prev in INR                  194,004.29 INR
```

## What it understands

| | |
|---|---|
| Arithmetic | `2^10`, `sqrt(16)`, `10 mod 3`, `5 x 3`, `10 times 3`, `1.5k` |
| Percentages | `20% of 340`, `5% on 200`, `15% off $1,299`, `$120 as % of $480`, `5% of what is 6` |
| Units | `3.5 km in miles`, `2 hours + 45 min in minutes`, `72 fahrenheit in celsius`, `10 GB / 4` |
| Currency | `$30 in EUR`, `€50 + £20 in USD`, `₹500 in USD` (live rates, cached 12h) |
| Variables | `rent = $1200`, then `rent * 12` |
| Sheet words | `prev`, `sum`/`total`, `avg`/`average` (over the lines since the last blank line or header) |
| Dates | `today`, `now`, `tomorrow + 2 weeks`, `now - 90 minutes` |
| Time zones | `3pm PST in IST`, `time in Tokyo`, `now in London`, `10:30 London to New York`, `noon UTC in Bangalore` |
| Bases | `255 in hex`, `0xff in decimal`, `255 in binary` |
| Notes | `# Header`, `// comment`, `"quoted text"`, `Label: 40`, stray prose is ignored |

Math, units and currency come from [fend-core](https://github.com/printfn/fend); `src/engine.rs`
adds the Numi conventions on top.

## Using it

- `SUPER + ALT + =` opens Omacus (or focuses it), floating and centered.
- `Esc` or `Ctrl+Q` closes it. Nothing is ever lost: like Sublime Text, every sheet is saved as you
  type and reopens exactly where you left it, cursor included.
- Click a result to copy it; `Ctrl+Shift+C` copies the last result.
- `Ctrl` `+` / `-` zooms.
- Colors follow the active Omarchy theme (`colors.toml`) and update live on `omarchy theme set`.

### Sheets

| Key | |
|---|---|
| `Ctrl+N` | New sheet |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` (or `Ctrl+PgDn` / `PgUp`) | Next / previous sheet |
| `Ctrl+1` … `Ctrl+9` | Jump to sheet |
| `Alt+↑` / `Alt+↓` | Move sheet up / down |
| `Ctrl+Delete` | Delete sheet (moved to `trash/`, not destroyed) |
| `Ctrl+B` | Toggle the sidebar |

Right-click a sheet in the sidebar for the same actions. A sheet's title is its first line.
Sheets are plain text in `~/.local/share/omacus/sheets/`; order, cursors and the sidebar
state are in `~/.local/share/omacus/state.json`.

### Autocomplete

Suggestions for variables, units, keywords and time zones appear as you type.
`Tab` accepts, `↑`/`↓` choose, `Esc` dismisses. `Enter` always inserts a newline.

CLI:

```sh
omacus '$100 in INR'
printf 'a = 5\na * 3\n' | omacus -
```

## Install

Omacus runs on Linux (Omarchy, Arch, Ubuntu, Fedora…), Windows and macOS.

**Downloads:** each tagged release on GitHub has
`omacus-linux-x86_64.tar.gz`, `omacus-linux-x86_64.deb`, `omacus-windows-x86_64.zip` and
`omacus-macos-universal.dmg` (Apple Silicon and Intel), built by `.github/workflows/release.yml`.

- **Ubuntu / Debian:** `sudo apt install ./omacus-linux-x86_64.deb`
- **Other Linux:** unpack the `.tar.gz` and run `./install.sh`, or build from source (below).
- **Windows:** unzip and run `omacus.exe`. It also works as a CLI: `omacus.exe "$100 in INR"`.
- **macOS:** open the `.dmg` and drag Omacus to Applications. The app isn't notarized yet, so the
  first time, right-click it and choose **Open** (or run `xattr -dr com.apple.quarantine /Applications/Omacus.app`).

**From source** (any platform, with [Rust](https://rustup.rs)):

```sh
cargo build --release          # binary in target/release/
./install.sh                   # Linux: installs to ~/.local/bin with a launcher entry and icon
```

### Where things live

| | Linux | macOS | Windows |
|---|---|---|---|
| Sheets | `~/.local/share/omacus` | `~/Library/Application Support/omacus` | `%APPDATA%\omacus` |
| Custom theme | `~/.config/omacus/theme.toml` | `~/Library/Application Support/omacus/theme.toml` | `%APPDATA%\omacus\theme.toml` |

Colors: your `theme.toml` if present (Omarchy `colors.toml` format), else the active Omarchy theme,
else the system's light or dark mode. On macOS, shortcuts use ⌘ wherever this README says Ctrl.

### Omarchy / Hyprland

`SUPER + ALT + =` is a keybind in `~/.config/hypr/bindings.lua`, and the float rule is in
`~/.config/hypr/looknfeel.lua`:

```lua
o.bind("SUPER + ALT + EQUAL", "Omacus calculator", { launch = "omacus", focus = "^omacus$" })
o.window("^omacus$", { float = true, size = { 980, 600 }, center = true })
```

On other systems, bind a global shortcut to `omacus` in your desktop's keyboard settings.
