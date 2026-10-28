THIS PROJECT WAS MADE WITH AI FOR PERSONAL USE!

# Dynamic Island for Windows

A lightweight "dynamic island" for Windows written in Rust: a black pill anchored to the top edge of the screen that expands on hover and shows music with synchronized lyrics, clock with weather, calendar, system stats, pomodoro/timer/stopwatch, calculator, tasks, and screen capture controls.

> **Status:** in development. Intended for Windows 10/11.

## Features

- **Music**: title, artist, cover art, progress bar (click to seek), previous / play-pause / next, an animated visualizer, and synchronized lyrics from LRCLIB shown right below the controls. Works with any player that integrates with Windows media controls (Spotify, browsers, etc.). The lyrics are hidden by default: a lyrics icon next to the controls turns them on or off (and, with *Lyric in the island* enabled, also shows the current line in the collapsed island).
- **Clock**: a compact row with the time, date and battery on the left and the current weather (icon, temperature, condition and city) on the right.
- **Calendar**: month view with today highlighted.
- **Weather**: current conditions and a 3-day forecast via Open-Meteo (no API key). City search, IP-based location detection, and °C/°F units. Refreshes every 15 minutes while visible.
- **System**: CPU, RAM, disk free space, network throughput, uptime, and 60-sample sparklines.
- **Time**: pomodoro, timer, and stopwatch in a single tab.
  - **Pomodoro**: 25-minute focus sessions, 5-minute short breaks, and a 15-minute long break every 4 rounds. Start/pause, reset, and skip.
  - **Timer**: 1 / 5 / 10 / 25-minute presets, pause, reset, and +1 minute. The countdown is also visible when the island is collapsed.
  - **Stopwatch**: tenths of a second and laps.
- **Calculator**: clickable keys, parentheses, and a live result. Accepts keyboard input when the window has focus.
- **Tasks**: a simple to-do list stored locally.
- **Battery alerts**: when the charger is connected/disconnected and when the battery drops below 20%.
- **Volume**: mouse wheel over the island; middle-click to mute.
- **Volume mixer**: master volume and per-application volume with mute buttons (Core Audio). Click an app's name (or right-click its name/mute icon) to choose which output device that app plays through.
- **Media source**: a chip next to the controls cycles between the apps that expose media controls (Spotify, browsers, foobar2000, ...); *Auto* follows the system's current session.
- **Hide in fullscreen** (games, videos).
- **Dodge browser tabs** (optional): hold the mouse on the top edge of the screen over the island for one second and it slides down out of the way; it returns when the cursor moves away. It also works in Notch style: the notch detaches and becomes a floating island while it dodges (the very top row of pixels does not expand the notch, so you can park the cursor there).
- **Screen capture**:
  - `Ctrl+Alt+S` takes a screenshot of the entire virtual desktop (all monitors) and saves it as PNG.
  - `Ctrl+Alt+R` starts/stops a 30 fps H.264/MP4 recording of the primary monitor.
  - The island hides itself during capture/recording by default.
- **Tray icon** with a menu for settings, capture actions, and exit.
- **Settings**: every feature can be enabled or disabled from the *Settings* tab or the tray menu.

## Usage

Hover over the island to expand it and use the tab bar (**Music · Clock · Calendar · Weather · Time · Calc · Tasks · System · Capture · Settings**). The dot on the far right keeps it open (*pin*). Right-click the island or the tray icon to open the menu.

Configuration is stored in `%APPDATA%\DynamicIsland\config.ini`.
Tasks are stored in `%APPDATA%\DynamicIsland\todo.txt`.

### Screenshots and recordings

- Screenshots: `%USERPROFILE%\Pictures\DynamicIsland\Captura_YYYY-MM-DD_HH-MM-SS.png`
- Recordings: `%USERPROFILE%\Videos\DynamicIsland\Grabacion_YYYY-MM-DD_HH-MM-SS.mp4`

Both folders can also be opened from the tray menu.

### Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl+Alt+S` | Take a screenshot |
| `Ctrl+Alt+R` | Start/stop screen recording |

## Download

Prebuilt executables are available in the repository's **Releases** section (if a release has been published).

## Build

Requirements: [Rust](https://rustup.rs) and the Visual Studio C++ Build Tools.

```powershell
git clone <repo-url>
cd DynamicIsland
cargo build --release
.\target\release\dynamic_island.exe
```

The executable is placed at `target\release\dynamic_island.exe`. To exit, right-click the island or the tray icon and choose *Exit*.

## How it works

- A single Win32 layered window with per-pixel alpha; no UI framework.
- Software rendering with anti-aliased edges (signed distance fields) and GDI text.
- Animation is a spring synchronized with the desktop compositor (DWM) and runs only while something is moving; idle CPU usage is essentially zero.
- Media information and controls use the Windows media transport controls (WinRT).
- Weather uses native WinHTTP with TLS 1.2/1.3 and Open-Meteo.
- Screenshots use GDI `BitBlt` and the Windows Imaging Component (WIC) PNG encoder.
- Recording uses GDI capture and Media Foundation (`IMFSinkWriter`) with the H.264 encoder.
- No third-party capture or recording tools are used.

## Limitations

- Recording captures the **primary monitor only** and does not record audio.
- Screenshots capture the full virtual desktop (all monitors).
- Recording requires an available H.264 encoder (included with Windows 10/11).
- Weather requires an internet connection; IP-based location is approximate.
- The UI is designed for the top edge of the primary monitor.

## Publishing a release

The `.github/workflows/build.yml` workflow builds on every push and, when a tag starting with `v` is pushed, attaches the `.exe` to a release:

```powershell
git tag v0.7.0
git push origin v0.7.0
```

## License

[MIT](LICENSE)
