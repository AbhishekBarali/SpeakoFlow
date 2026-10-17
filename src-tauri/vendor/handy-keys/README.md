# handy-keys

Cross-platform global keyboard shortcuts library for Rust.

## Features

- **Cross-platform**: Works on macOS, Windows, and Linux
- **Global hotkeys**: Register system-wide keyboard shortcuts
- **Hotkey blocking**: Registered hotkeys are blocked from reaching other applications
- **Modifier-only hotkeys**: Support for shortcuts like `Cmd+Shift` without a key
- **String parsing**: Parse hotkeys from strings like `"Ctrl+Alt+Space"`
- **Hotkey recording**: Low-level keyboard listener for "record a hotkey" UI flows
- **Serde support**: All types implement `Serialize`/`Deserialize`

## Installation

```toml
[dependencies]
handy-keys = "0.1"
```

## Quick Start

```rust
use handy_keys::{HotkeyManager, Hotkey, Modifiers, Key};

fn main() -> handy_keys::Result<()> {
    let manager = HotkeyManager::new()?;

    // Register using the type-safe constructor
    let hotkey = Hotkey::new(Modifiers::CMD | Modifiers::SHIFT, Key::K)?;
    let id = manager.register(hotkey)?;

    // Or parse from a string
    let hotkey2: Hotkey = "Ctrl+Alt+Space".parse()?;
    manager.register(hotkey2)?;

    // Listen for events
    while let Ok(event) = manager.recv() {
        println!("Hotkey triggered: {:?}", event.id);
    }

    Ok(())
}
```

## Platform Notes

### macOS

Requires accessibility permissions. The library provides helpers to check and request access:

```rust
use handy_keys::{check_accessibility, open_accessibility_settings};

if !check_accessibility() {
    open_accessibility_settings()?;
}
```

### Windows

Uses low-level keyboard hooks. No special permissions required.

### Linux

Reads the input devices through evdev, so it works the same under X11 and any Wayland compositor. Reading needs access to `/dev/input/event*` (the `input` group). Withholding a hotkey's key from the focused app takes the keyboard over and re-sends everything else through a uinput mirror, which needs write access to `/dev/uinput`; without it, hotkeys still fire but their key also reaches the app. A keyboard is only taken over while no key on it is held, and a device appearing or going away never stops capture.

## Modifiers

| Modifier | Aliases                           |
| -------- | --------------------------------- |
| `CMD`    | `command`, `meta`, `super`, `win` |
| `CTRL`   | `control`                         |
| `OPT`    | `option`, `alt`                   |
| `SHIFT`  |                                   |
| `FN`     | `function` (macOS only)           |

## Recording Hotkeys

For implementing "press a key to set hotkey" UI:

```rust
use handy_keys::KeyboardListener;

let listener = KeyboardListener::new()?;

println!("Press a key combination...");
while let Ok(event) = listener.recv() {
    if event.is_key_down {
        if let Ok(hotkey) = event.as_hotkey() {
            println!("Recorded: {}", hotkey);
            break;
        }
    }
}
```

## License

MIT
