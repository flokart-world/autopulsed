// autopulsed - A daemon for configuring PulseAudio automatically
// Copyright (C) 2025  Flokart World, Inc.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

/// Isolated PulseAudio server for testing
struct IsolatedPulseServer {
    process: Option<Child>,
    temp_dir: TempDir, // Kept alive until Drop to preserve temp files
    socket_path: PathBuf,
}

impl IsolatedPulseServer {
    fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let temp_dir = TempDir::new()?;
        let runtime_dir = temp_dir.path().join("runtime");
        let state_dir = temp_dir.path().join("state");
        let socket_path = runtime_dir.join("pulse.sock");

        std::fs::create_dir_all(&runtime_dir)?;
        std::fs::create_dir_all(&state_dir)?;

        // Create a minimal PulseAudio config
        let config_path = temp_dir.path().join("pulse.conf");
        let socket_str = socket_path.display().to_string();
        std::fs::write(&config_path,
            format!("daemonize = no
exit-idle-time = -1
flat-volumes = no
default-sample-format = s16le
default-sample-rate = 44100
default-sample-channels = 2

# Only load minimal modules for testing
load-module module-native-protocol-unix socket={}
load-module module-null-sink sink_name=test_sink_1 sink_properties=device.description=TestSink1
load-module module-null-sink sink_name=test_sink_2 sink_properties=device.description=TestSink2
", socket_str))?;

        // Start isolated PulseAudio instance
        let process = Command::new("pulseaudio")
            .args(&[
                "--daemonize=no",
                "--use-pid-file=no",
                "--system=no",
                "--disallow-exit=yes",
                "--exit-idle-time=-1",
                "--file",
                config_path.to_str().unwrap(),
            ])
            .env("PULSE_RUNTIME_PATH", &runtime_dir)
            .env("PULSE_STATE_PATH", &state_dir)
            .env("PULSE_CONFIG_PATH", temp_dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        // Wait for server to start
        thread::sleep(Duration::from_millis(500));

        Ok(Self {
            process: Some(process),
            temp_dir,
            socket_path,
        })
    }

    fn socket_path(&self) -> String {
        format!("unix:{}", self.socket_path.display())
    }
}

impl Drop for IsolatedPulseServer {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            // Try graceful shutdown first
            process.kill().ok();
            process.wait().ok();
        }
    }
}

#[test]
fn test_device_enumeration_with_mock_pulse() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    let server = IsolatedPulseServer::start()
        .expect("Failed to start isolated PulseAudio server");

    // Create a test config file with both sinks and sources
    // Note: null-sink devices might not have many properties, so we use device.description
    let config_content = r#"
sinks:
  test_device_1:
    priority: 1
    detect:
      device.description: "TestSink1"
  test_device_2:
    priority: 2
    detect:
      device.description: "TestSink2"

sources:
  test_monitor_1:
    priority: 1
    detect:
      device.description: "Monitor of TestSink1"
  test_monitor_2:
    priority: 2
    detect:
      device.description: "Monitor of TestSink2"
"#;

    let config_path = server.temp_dir.path().join("test_config.yml");
    std::fs::write(&config_path, config_content)
        .expect("Failed to write test config");

    // Start autopulsed with our test config and PulseAudio server
    let mut autopulsed = Command::new("cargo")
        .args(&[
            "run",
            "--",
            "--config",
            config_path.to_str().unwrap(),
            "--verbose",
        ])
        .env("PULSE_SERVER", server.socket_path())
        .env("RUST_LOG", "debug")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start autopulsed");

    // Read stderr for log output (env_logger outputs to stderr)
    let stderr = autopulsed.stderr.take().expect("Failed to get stderr");
    let reader = BufReader::new(stderr);

    let mut found_connected = false;
    let mut found_test_sink_1 = false;
    let mut found_test_sink_2 = false;
    let mut found_test_monitor_1 = false;
    let mut found_test_monitor_2 = false;
    let mut found_sink_detected = false;
    let mut found_source_detected = false;
    let mut found_default_sink = false;
    let mut found_default_source = false;

    // Read logs for a few seconds
    let start = std::time::Instant::now();
    for line in reader.lines() {
        if start.elapsed() > Duration::from_secs(3) {
            break;
        }

        if let Ok(line) = line {
            println!("LOG: {}", line);

            if line.contains("Connected to PulseAudio server") {
                found_connected = true;
            }
            // Check for device discovery
            if line.contains("Found sink") && line.contains("test_sink_1") {
                found_test_sink_1 = true;
            }
            if line.contains("Found sink") && line.contains("test_sink_2") {
                found_test_sink_2 = true;
            }
            if line.contains("Found source")
                && line.contains("test_sink_1.monitor")
            {
                found_test_monitor_1 = true;
            }
            if line.contains("Found source")
                && line.contains("test_sink_2.monitor")
            {
                found_test_monitor_2 = true;
            }
            // Check for device detection (matching config)
            if line.contains("Sink")
                && line.contains("detected as 'test_device_1'")
            {
                found_sink_detected = true;
            }
            if line.contains("Source")
                && line.contains("detected as 'test_monitor_1'")
            {
                found_source_detected = true;
            }
            // Check for default device setting
            if line.contains("Using sink 'test_device_1' as default") {
                found_default_sink = true;
            }
            if line.contains("Using source 'test_monitor_1' as default") {
                found_default_source = true;
            }
        }
    }

    // Kill autopulsed
    autopulsed.kill().ok();
    autopulsed.wait().ok();

    // Verify basic connectivity
    assert!(found_connected, "Should connect to PulseAudio");

    // Verify device discovery
    assert!(found_test_sink_1, "Should find test_sink_1");
    assert!(found_test_sink_2, "Should find test_sink_2");
    assert!(found_test_monitor_1, "Should find test_sink_1.monitor");
    assert!(found_test_monitor_2, "Should find test_sink_2.monitor");

    // Verify device detection (config matching)
    assert!(
        found_sink_detected,
        "Should detect sink as configured device"
    );
    assert!(
        found_source_detected,
        "Should detect source as configured device"
    );

    // Verify default device setting
    assert!(
        found_default_sink,
        "Should set default sink to priority 1 device"
    );
    assert!(
        found_default_source,
        "Should set default source to priority 1 device"
    );
}
