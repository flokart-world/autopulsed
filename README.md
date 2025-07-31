# autopulsed

A daemon for adjusting PulseAudio settings automatically based on device connectivity changes, written in Rust.

## Features

### Currently implemented

- YAML-based configuration
- Real-time monitoring of audio device connections and disconnections
- Automatic default device switching

### Planned for future

- Automatic remap device creation
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

## Configuration example

```yaml
sinks:
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
    priority: 1
    detect:
      device.bus: "usb"
      device.serial: "Focusrite_Scarlett_2i2_4th_Gen_XXXXXXXXXXXXXX"
```
