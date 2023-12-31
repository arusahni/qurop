# Qurop

Turn any* Linux app into a Quake-style dropdown.

\* only tested with Wezterm


**Only X11 for now, but I'd love to support Wayland, too**

## Getting started

### Installation

Cargo is the recommended method: `cargo install qurop`

Binaries and distro packages coming soon.

## Configuration

Assuming `~/.cargo/bin` is on your path, register your application with Qurop:

```sh
# Add an instance named "wezterm", which, when invoked, will launch Wezterm
#  using the "dropdown" session.
qurop add wezterm wezterm connect dropdown
```

This updates the configuration located in `$XDG_CONFIG_HOME/qurop/config.toml`.

Next, within your desktop environment, bind the toggle command to a shortcut:

```sh
# Toggle the visibility of the "wezterm" instance.
/home/$USER/.cargo/bin/qurop toggle wezterm
```

(in KDE, this is System Settings > Shortcuts > Custom Shortcuts > Edit > New >
Global Shortcut > Command/URL)

Then, hit the shortcut! That should be it.

### Window size

The window defaults to 66% wide and 50% high. This can be altered by editing
the config file to add the `geometry` key to the table for your application
instance:

```toml
geometry = { width = "<val>", height = "<val>" }
```

Values must be strings, and can contain either absolute numbers (e.g., `"55"`
for 55 pixels), or percentages (e.g., `"33%"`).

### Matchers

By default, Qurop tracks the state of a managed application instance by it's
PID. In the event a PID is *not* the right way to locate the instance window,
you can use an alternate matcher to find the window. Currently the only
supported alternate matcher uses the window class.

```sh
# Add an instance named "my_instance", which, when invoked, will launch
#  "appname". Qurop will detect this by querying X for the first window
#  with an "appclass" class.
qurop add --matcher class --class-name appclass my_instance appname
```

## Troubleshooting

### The window isn't resized or placed correctly

Qurop resizes the window after it launches. Depending on how long it takes to
launch, the window may not be ready in time. You can alter the default delay
(100ms) by adding the `window_delay_ms` key to the table for your application
instance:

```toml
window_delay_ms = 500
```

### Logs
To enable logging run the program with the `QUROP_LOG` envvar set:

```sh
QUROP_LOG=debug qurop toggle wezterm
```

This will output to the attached TTY. If running outside of a terminal session,
you can opt into having logs written to disk:

```sh
QUROP_LOG=debug qurop --persist-verbosity=debug toggle wezterm
```

This will output hourly-rotating logs to `/home/$USER/.local/state/qurop/`.


## On the roadmap

I'd love to support the following. If you'd like to contribute a feature,
please reach out to discuss an implementation before actually writing the code!

- [ ] Wayland support. This will be challenging due to needing to detect the
  active window, which doesn't yet seem to be supported by any protocol.
- [ ] System tray icon.
- [ ] Show/hide animations.
- [ ] Better error handling.
- [ ] Additional matchers.
