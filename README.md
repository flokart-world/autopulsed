# autopulsed

A daemon for adjusting PulseAudio settings automatically based on device connectivity changes, written in Rust.

## Features

### Currently implemented

- YAML-based configuration
- Real-time monitoring of audio device connections and disconnections
- Automatic default device switching
- Automatic remap device creation and removal based on master device availability
- Circular reference detection in remap configurations

### Planned for future

- Hot-reloading configuration

## Building

libpulse-dev and the development environment for Rust are needed.

```bash
cargo build --release
```

## Usage

Setting up as a systemd service is recommended.

### Command line options

See help.

```bash
autopulsed --help
```

### Systemd service setup

Example systemd user service file `~/.config/systemd/user/autopulsed.service`:

```ini
[Unit]
Description=Automatic PulseAudio device management daemon
# For PipeWire users, use pipewire-pulse.service instead of pulseaudio.service
After=pulseaudio.service
Requires=pulseaudio.service

[Service]
Type=simple
ExecStart=/usr/local/bin/autopulsed --config %h/.config/autopulsed/config.yml
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
```

## Configuration example

```yaml
sinks:
  # Regular device detection
  hdmi:
    priority: 2
    detect:
      device.bus: "pci"
      device.bus_path: "pci-0000:01:00.1"
  iec958:
    priority: 3
    detect:
      device.bus: "usb"
      device.bus_path: "pci-0000:14:00.0-usb-0:8:1.0"
  scarlett:
    priority: 1
    detect:
      device.bus: "usb"
      device.serial: "Focusrite_Scarlett_2i2_4th_Gen_XXXXXXXXXXXXXX"

sources:
  iec958:
    priority: 2
    detect:
      device.bus: "usb"
      device.bus_path: "pci-0000:14:00.0-usb-0:8:1.0"
  scarlett:
    detect:
      device.bus: "usb"
      device.serial: "Focusrite_Scarlett_2i2_4th_Gen_XXXXXXXXXXXXXX"
      media.class: "Audio/Source"

  # Remap device - creates mono source from Scarlett's first channel
  scarlett_mono:
    priority: 1
    remap:
      master: "scarlett"  # Reference to another device name
      device_name: "scarlett_mono"
      device_properties:
        device.description: "Scarlett 2i2 4th Gen (Mono Ch1)"
      channels: 1
      channel_map: "mono"
      master_channel_map: "front-left"
```

### Configuration options

#### Device detection (`detect`)
Matches devices based on PulseAudio properties:
- `device.bus`: Device bus type (e.g., "pci", "usb")
- `device.bus_path`: Bus path identifier
- `device.serial`: Device serial number
- Any other PulseAudio device property

#### Remap devices (`remap`)
Creates virtual devices using PulseAudio's remap modules:
- `master`: Name of the master device (must be defined in the same configuration)
- `device_name`: Name for the remapped device
- `device_properties`: Key-value pairs for device properties (e.g., `device.description: "My Device"`)
- `format`: Audio format (e.g., "s16le", "float32le")
- `rate`: Sample rate (e.g., 44100, 48000)
- `channels`: Number of channels
- `channel_map`: Channel mapping (e.g., "front-left,front-right")
- `master_channel_map`: Master device channel mapping
- `resample_method`: Resampling method
- `remix`: Enable remixing (true/false)

Remap devices are automatically created when their master device appears and removed when the master device disappears.
