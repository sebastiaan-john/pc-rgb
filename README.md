# pc-rgb

A tiny, fast replacement for SignalRGB if all you want is colors. It's one ~540 KB native
Windows exe that sits in the tray and talks to the lighting hardware directly. There's no
browser engine, Qt, background service, plugin store, account, or telemetry.

| | SignalRGB | pc-rgb |
|---|---|---|
| Memory | ~1.1 GB | ~17 MB |
| CPU, solid color | – | 0% |
| CPU, animated effect | – | ~4.5% of one core (~0.3% of a 16-thread CPU) |
| Install size | ~2.5 GB | 540 KB |

*Measured on a Ryzen / X870 AORUS ELITE / RTX 5090 / Corsair DDR5 system.*

## Supported hardware

pc-rgb only supports a small set of devices. It was built for one specific PC, and the
device code is written so more devices can be added.

| Device | Status | How |
|---|---|---|
| Gigabyte motherboards with the **ITE IT5711** RGB controller (USB `048d:5711`, e.g. X870 / B850 / Z890 AORUS): onboard zones + 3 ARGB headers | tested on X870 AORUS ELITE WIFI7 | USB HID |
| **MSI RTX 50-series** GPUs: GAMING TRIO, SUPRIM, VANGUARD (5060–5090) | tested on 5090 GAMING TRIO | NVIDIA I2C via `nvapi64.dll` |
| **Corsair DDR5 RGB** (Dominator Platinum, Vengeance RGB) on **AMD** chipsets | tested on 2× CMH64GX5M2B6000Z30 | AMD PIIX4 SMBus via [PawnIO](https://pawnio.eu) |

Not supported (yet): other motherboard vendors, Intel SMBus (RAM on Intel systems), keyboards,
mice, headsets, AIOs, and other GPU vendors. PRs are welcome. Each device family is one
small file in `src/devices/`.

The protocols are reimplemented from the public SignalRGB device plugins and OpenRGB.

## Effects

Solid color · Pastel (a soft hue wave, like SignalRGB's "Pastel") · Rainbow · Breathing · Off.

Each LED's color is computed straight from its position on a 320×200 canvas, so nothing is
ever rendered. Static colors sleep and only re-send every 5 s. Animated effects run at
20 fps by default.

## Install

1. Install [PawnIO](https://pawnio.eu) if you want RAM lighting. It's a signed driver, and
   OpenRGB / LibreHardwareMonitor use it too.
2. **Uninstall SignalRGB / OpenRGB / iCUE / RGB Fusion**, or at least make sure they aren't
   running. Two apps driving the same controller will fight. Also turn off Windows
   *Dynamic Lighting* for the motherboard.
3. Download `pc-rgb.exe` (or build it, see below), put it somewhere permanent (e.g.
   `%LOCALAPPDATA%Programspc-rgb`), and run it. It asks for admin once (needed for RAM).
   If you decline, the motherboard and GPU still work. On first launch it adds a **pc-rgb**
   Start Menu shortcut, so you can find it again with Windows search.
4. Click the tray icon and turn on **Start with Windows**. That creates a logon task with
   highest privileges, so there's no UAC prompt at boot.

`pc-rgb.exe --probe` detects devices without writing anything and logs the result to
`%APPDATA%\pc-rgb\probe.log`.

## Configuration

Everything is in the tray menu, or in `%APPDATA%\pc-rgb\config.ini`, which reloads
automatically when you save it:

```ini
effect = pastel          # solid | pastel | rainbow | breathing | off
color = #009bde
pastel_color = #5050af
speed = 20               # 0-100
brightness = 100
fps = 20
header1 = none           # what's on each 5V ARGB header: none | ml_fan | <LED count>
header2 = ml_fan
header3 = 60
pos.gpu = 29, 112, 25.9, 8.2   # canvas placement: x, y, cell width, cell height
```

The defaults (header contents, canvas positions) match the author's PC. Change the `header*`
lines to match your fans and strips, and adjust `pos.*` if you care about how waves line up.

Logs are in `%APPDATA%\pc-rgb\pc-rgb.log`, including per-device fps and CPU every 30 s.

## Build

```sh
rustup default stable-x86_64-pc-windows-gnu   # no Visual Studio needed
cargo build --release                          # -> target/release/pc-rgb.exe
cargo test
```

## License

MIT. `vendor/pawnio/SmbusPIIX4.bin` is the signed SMBus module from
[PawnIO.Modules](https://github.com/namazso/PawnIO.Modules) 0.2.11 (LGPL-2.1, see
`vendor/pawnio/COPYING`).

**Disclaimer:** this writes directly to SMBus/I2C and USB lighting controllers. It uses the
same commands as the established tools, but you use it at your own risk.
