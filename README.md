# uwoniri

Dynamic window resizing for [niri](https://github.com/YaLTeR/niri) on ultrawide monitors

By default, niri sets the width of every new window to a static, predefined width. This usually works great on a standard width monitor. On ultrawide monitors, it can quickly become awkward:
as windows are added and removed, their distribution can become lopsided across the monitor and way off the user's central field of view, requiring them to constantly resize and center the columns.
`uwoniri` is a small background daemon that watches your workspaces, and resizes and centers columns according to a simple mechanism: Individual window widths are configured according to how many windows
there are in a workspace instead.

`uwoniri` is a spiritual sibling of [oniri](https://github.com/Antiz96/oniri), which maximizes the only window of a niri workspace.

## Contents

- [Behavior](#behavior)
- [Installation](#installation)
- [Usage](#usage)
- [Configuration](#configuration)
- [Logging and troubleshooting](#logging-and-troubleshooting)
- [Limitations](#limitations)

## Behavior

Whenever a window opens, closes, or moves to another workspace, `uwoniri` re-applies a defined layout for the workspace involved.

The layout philosophy is build around a _centered_ _main window_, surrounded by any number of _side windows_.

Default layouts:

| Windows   | Main window | Other windows                                              |
| --------- | ----------- | ---------------------------------------------------------- |
| 1         | 50%         | n/a                                                        |
| 2         | 50%         | 25%, to the right                                          |
| 3         | 40%         | 30%, on either side of the main window                     |
| 4 or more | n/a         | newly opened windows: 33%; existing windows are left alone |

Past the point where a layout is defined (three windows, by default) the daemon deliberately stops rearranging your workspace. Any new window created past this point are sized to an _overflow width_ (33% by default), but already open windows are not resized.

You can rearrange the layout, and define a new main window using the [`retile`](#sending-commands-to-the-daemon) command.

Some other things worth knowing:

- Floating windows are ignored.
- `uwoniri` can be limited to specific monitors. Ignored monitors respect niri's normal behavior. See [Configuration](#configuration).
- The layouts are fully configurable.

## Installation

### Nix flake

Add the following to your flake's inputs:

```nix
inputs.uwoniri.url = "github:jbeaurivage/uwoniri";
```

To install it system-wide, add the flake as an input and use either its `packages.<system>.default` or the provided `overlays.default`, which adds `pkgs.uwoniri`.

### Installing from `crates.io`

```sh
cargo install uwoniri
```

### Installing from source

You'll need a recent Rust toolchain (1.88 or newer)

```sh
cargo install --git https://github.com/jbeaurivage/uwoniri
```

### Compatibility with niri

`uwoniri` talks to niri through the [`niri-ipc`](https://crates.io/crates/niri-ipc) crate, whose versions follow niri's own. The IPC is not guaranteed to be compatible across niri versions: the chart below
outlines the compatibility with niri versions. Other combinations may also work, but are untested.

#### Compatibility chart

| uwoniri | niri  |
| ------- | ----- |
| 0.2.0   | 26.04 |

## Usage

### Running the daemon

Start it from your niri configuration (`~/.config/niri/config.kdl`):

```kdl
spawn-at-startup "uwoniri"
```

Only one instance can run at a time. Starting a second one exits right away with an error.

### Sending commands to the daemon

`uwoniri msg` talks to an already running daemon:

```sh
# stop adjusting layouts automatically
uwoniri msg pause
 # start adjusting them again
 uwoniri msg unpause
# re-apply layout, making the focused window the main window
uwoniri msg retile
```

- **`pause`/`unpause`**: while paused, layouts are not applied automatically; however the retile command still works. Unpausing does not re-tile anything by itself. By default, uwoniri starts paused, unless the `-d` command-line flag is used.
- **`retile`**: the focused window becomes the main window, and the layout is reapplied around it. When there are more windows than the largest layout covers, the focused window and the neighbors needed to fill that layout follow it, and every window further away gets the overflow width.

You can configure keybinds for uwoniri in niri's `config.kdl`, for example:

```kdl
binds {
    Mod+Shift+R { spawn "uwoniri" "msg" "retile"; }
    Mod+Shift+P { spawn "uwoniri" "msg" "pause"; }
    Mod+Shift+U { spawn "uwoniri" "msg" "unpause"; }
}
```

Under the hood, `uwoniri msg` to a Unix socket at `$XDG_RUNTIME_DIR/uwoniri.sock`. Valid socket commands are simply the command name, followed by a newline, ie:

#### Valid socket commands

```sh
pause
unpause
retile
```

### Command line reference

```text
$ uwoniri --help
A tool that automatically maximizes the only window of a niri workspace

Usage: uwoniri [OPTIONS] [COMMAND]

Commands:
  msg   Send a command to the running uwoniri instance
  help  Print this message or the help of the given subcommand(s)

Options:
  -d, --dynamic  Dynamically retile as windows are added or removed in the workspade (disabled by default)
  -h, --help     Print help
  -V, --version  Print version
```

## Configuration

The configuration file is optional. It lives at `$XDG_CONFIG_HOME/uwoniri/config.toml` (usually `~/.config/uwoniri/config.toml`) and is read once at startup. The daemon should be restarted to apply changes
to the configuration.

```toml
# Which monitors to manage. This setting is optional.
[watch]
outputs   = ["DP-2"]   # by connector name (see `niri msg outputs`)
min_width = 3440       # or any monitor at least this many logical pixels wide

# When neither setting is defined, every monitor is managed. With both, a monitor
# qualifies if it matches either one.

[widths]
overflow = 33.0        # width of new windows beyond the last layout below

# Entry number N is used when a workspace has N windows.
layouts = [
  { main = 50.0 },                  # 1 window
  { main = 50.0, side = 25.0 },     # 2 windows
  { main = 40.0, side = 30.0 },     # 3 windows
  # { main = 34.0, side = 22.0 },   # uncomment to manage 4 windows, and so on
]
```

Things to know:

- All widths are percentages and must be greater than 0 and at most 100.
- `main` is the width of the main window, `side` the width of the others in that layout. Every entry after the first needs to specify a `side` width.
- Unknown keys in `[widths]` are rejected.
- If the file can't be read or is invalid, `uwoniri` logs why and runs with the defaults shown above.
- If the file doesn't exist, the defaults apply to all monitors.

## Logging and troubleshooting

Logs are written to stderr and are controlled by the `RUST_LOG` environment variable. By default only errors are shown. To see what the daemon is doing, start it like this:

```kdl
spawn-at-startup "env" "RUST_LOG=uwoniri=info" "uwoniri"
```

Use `uwoniri=debug` for more detail. When niri is started through `niri-session`, stderr of programs it spawns goes to the systemd journal, so something like `journalctl --user -u niri -f` is where you'll find it.

If `uwoniri msg` says it can't connect, the daemon isn't running (or is running under a different `XDG_RUNTIME_DIR`).

## Limitations

- niri's IPC only reports that a window was "opened or changed", so `uwoniri` keeps its own record of which windows live on which workspace to tell the two apart. In unusual setups this can occasionally get out of step with niri.
- Windows stacked in the same column are counted as separate windows.
- A window that's moved between workspaces or toggled to floating is handled, but only once niri tells the daemon about it, so there can be a brief delay.
- Window sizes are changed through niri's normal actions, so clients that enforce a minimum or fixed size may not end up at the exact percentage requested.
