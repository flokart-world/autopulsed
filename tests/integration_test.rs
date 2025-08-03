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

        // Create a minimal PulseAudio script (not .conf but .pa format)
        let config_path = temp_dir.path().join("pulse.pa");
        let socket_str = socket_path.display().to_string();
        std::fs::write(&config_path,
            format!("#!/usr/bin/pulseaudio -nF
# Minimal PulseAudio configuration for testing

# Load only the necessary modules
.fail
load-module module-native-protocol-unix socket={} auth-anonymous=1
load-module module-null-sink sink_name=test_sink_1 sink_properties=device.description=TestSink1
load-module module-null-sink sink_name=test_sink_2 sink_properties=device.description=TestSink2
", socket_str))?;

        // Start isolated PulseAudio instance
        eprintln!("TEST: Starting PulseAudio with socket at: {}", socket_str);
        eprintln!("TEST: Config file: {}", config_path.display());
        let mut process = Command::new("pulseaudio")
            .args(&[
                "-n", // Don't load default script to avoid conflicts
                "--daemonize=no",
                "--use-pid-file=no",
                "--system=no",
                "--disallow-exit=yes",
                "--exit-idle-time=-1",
                "--disable-shm", // Disable shared memory
                "--file",
                config_path.to_str().unwrap(),
            ])
            .env("PULSE_RUNTIME_PATH", &runtime_dir)
            .env("PULSE_STATE_PATH", &state_dir)
            .env("PULSE_CONFIG_PATH", temp_dir.path())
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/dev/null") // Disable session D-Bus
            .env("DBUS_SYSTEM_BUS_ADDRESS", "unix:path=/dev/null") // Disable system D-Bus
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // Check if process started successfully
        thread::sleep(Duration::from_millis(100));
        if let Ok(Some(status)) = process.try_wait() {
            return Err(format!(
                "PulseAudio process exited immediately with status: {:?}",
                status
            )
            .into());
        }

        // Wait for server to start by polling with pactl
        let socket_str_for_check = socket_str.clone();
        let mut server_ready = false;

        // First, check if the socket file exists
        eprintln!(
            "TEST: Checking for socket file at: {}",
            socket_path.display()
        );

        for i in 0..20 {
            // Try for up to 10 seconds
            // Check if socket file exists every 5 attempts
            if i % 5 == 0 {
                if socket_path.exists() {
                    eprintln!("TEST: Socket file exists at attempt {}", i + 1);
                } else {
                    eprintln!(
                        "TEST: Socket file does not exist yet at attempt {}",
                        i + 1
                    );
                }
            }

            let output = Command::new("pactl")
                .args(&["--server", &socket_str_for_check, "info"])
                .output();

            if let Ok(output) = output {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    // Check if we actually got server info (not just empty output)
                    if stdout.contains("Server String:")
                        || stdout.contains("Server Name:")
                    {
                        eprintln!(
                            "TEST: PulseAudio server ready after {} attempts",
                            i + 1
                        );
                        server_ready = true;
                        break;
                    } else {
                        eprintln!(
                            "TEST: pactl returned success but unexpected output: {}",
                            stdout
                        );
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    eprintln!(
                        "TEST: pactl check attempt {} failed: {}",
                        i + 1,
                        stderr
                    );
                }
            } else {
                eprintln!(
                    "TEST: pactl check attempt {} failed to execute",
                    i + 1
                );
            }
            thread::sleep(Duration::from_millis(500));
        }

        if !server_ready {
            // Get PulseAudio stderr before failing
            if let Some(mut stderr) = process.stderr.take() {
                use std::io::Read;
                let mut stderr_output = String::new();
                stderr.read_to_string(&mut stderr_output).ok();
                eprintln!("TEST: PulseAudio stderr output: {}", stderr_output);
            }
            return Err("PulseAudio server failed to become ready".into());
        }

        Ok(Self {
            process: Some(process),
            temp_dir,
            socket_path,
        })
    }

    fn socket_path(&self) -> String {
        let path = format!("unix:{}", self.socket_path.display());
        eprintln!("TEST: Using socket path: {}", path);
        path
    }
}

impl Drop for IsolatedPulseServer {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            eprintln!("TEST: Shutting down PulseAudio process");
            // Try graceful shutdown first
            if let Err(e) = process.kill() {
                eprintln!("TEST: Failed to kill PulseAudio process: {}", e);
            }
            // Wait for process to actually exit
            match process.wait() {
                Ok(status) => {
                    eprintln!(
                        "TEST: PulseAudio process exited with status: {:?}",
                        status
                    );
                }
                Err(e) => {
                    eprintln!(
                        "TEST: Failed to wait for PulseAudio process: {}",
                        e
                    );
                }
            }
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

    // Use timeout to prevent test hanging on reader.lines()
    let mut autopulsed = Command::new("timeout")
        .args(&[
            "2",
            "cargo",
            "run",
            "--",
            "--config",
            config_path.to_str().unwrap(),
            "--server",
            &server.socket_path(),
            "--verbose",
        ])
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
    eprintln!("TEST: Killing autopulsed process");
    if let Err(e) = autopulsed.kill() {
        eprintln!("TEST: Failed to kill autopulsed: {}", e);
    }
    if let Err(e) = autopulsed.wait() {
        eprintln!("TEST: Failed to wait for autopulsed: {}", e);
    }
    eprintln!("TEST: autopulsed process terminated");

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

#[test]
fn test_connection_to_nonexistent_server() {
    use std::process::Stdio;

    // Try to connect to /dev/null as server (guaranteed to fail)
    let output = Command::new("cargo")
        .args(&["run", "--", "--server", "/dev/null"])
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run autopulsed");

    // Should exit with error
    assert!(
        !output.status.success(),
        "Should fail to connect to /dev/null"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("STDERR: {}", stderr);

    // Should log connection error
    assert!(
        stderr.contains("Failed to connect to PulseAudio"),
        "Should report connection failure"
    );
}

#[test]
fn test_server_option_overrides_env() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    let server = IsolatedPulseServer::start()
        .expect("Failed to start isolated PulseAudio server");

    // Use timeout to prevent test hanging
    let mut autopulsed = Command::new("timeout")
        .args(&[
            "2",
            "cargo",
            "run",
            "--",
            "--server",
            &server.socket_path(),
            "--verbose",
        ])
        .env("PULSE_SERVER", "/dev/null") // This should be ignored
        .env("RUST_LOG", "debug")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start autopulsed");

    // Read stderr for log output
    let stderr = autopulsed.stderr.take().expect("Failed to get stderr");
    let reader = BufReader::new(stderr);

    let mut found_connected = false;

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
                break; // We only need to verify connection
            }
        }
    }

    // Kill autopulsed
    eprintln!("TEST: Killing autopulsed process");
    if let Err(e) = autopulsed.kill() {
        eprintln!("TEST: Failed to kill autopulsed: {}", e);
    }
    if let Err(e) = autopulsed.wait() {
        eprintln!("TEST: Failed to wait for autopulsed: {}", e);
    }
    eprintln!("TEST: autopulsed process terminated");

    // Should successfully connect using --server option, not the env var
    assert!(
        found_connected,
        "Should connect using --server option, overriding PULSE_SERVER env"
    );
}
