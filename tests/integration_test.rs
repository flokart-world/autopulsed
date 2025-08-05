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

mod helpers;

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

        // PulseAudio requires .pa format, not .conf
        let config_path = temp_dir.path().join("pulse.pa");
        let socket_str = socket_path.display().to_string();
        std::fs::write(&config_path,
            format!("#!/usr/bin/pulseaudio -nF
# Minimal PulseAudio configuration for testing

# Load only the necessary modules
.fail
load-module module-native-protocol-unix socket={socket_str} auth-anonymous=1
load-module module-null-sink sink_name=test_sink_1 sink_properties=device.description=TestSink1
load-module module-null-sink sink_name=test_sink_2 sink_properties=device.description=TestSink2
"))?;

        eprintln!("TEST: Starting PulseAudio with socket at: {socket_str}");
        eprintln!("TEST: Config file: {}", config_path.display());
        let mut process = Command::new("pulseaudio")
            .args([
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

        // PulseAudio may fail immediately if port is in use
        thread::sleep(Duration::from_millis(100));
        if let Ok(Some(status)) = process.try_wait() {
            return Err(format!(
                "PulseAudio process exited immediately with status: {status:?}"
            )
            .into());
        }

        // Poll with pactl instead of relying on process status
        let socket_str_for_check = socket_str.clone();
        let mut server_ready = false;

        eprintln!(
            "TEST: Checking for socket file at: {}",
            socket_path.display()
        );

        for i in 0..20 {
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
                .args(["--server", &socket_str_for_check, "info"])
                .output();

            if let Ok(output) = output {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    // pactl may return success with empty output during startup
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
                            "TEST: pactl returned success but unexpected output: {stdout}"
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
            if let Some(mut stderr) = process.stderr.take() {
                use std::io::Read;
                let mut stderr_output = String::new();
                stderr.read_to_string(&mut stderr_output).ok();
                eprintln!("TEST: PulseAudio stderr output: {stderr_output}");
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
        eprintln!("TEST: Using socket path: {path}");
        path
    }
}

impl Drop for IsolatedPulseServer {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            eprintln!("TEST: Shutting down PulseAudio process");
            if let Err(e) = process.kill() {
                eprintln!("TEST: Failed to kill PulseAudio process: {e}");
            }
            match process.wait() {
                Ok(status) => {
                    eprintln!(
                        "TEST: PulseAudio process exited with status: {status:?}"
                    );
                }
                Err(e) => {
                    eprintln!(
                        "TEST: Failed to wait for PulseAudio process: {e}"
                    );
                }
            }
        }
    }
}

#[test]
fn test_device_enumeration_with_mock_pulse() {
    use helpers::OutputCapturer;

    let server = IsolatedPulseServer::start()
        .expect("Failed to start isolated PulseAudio server");

    // null-sink devices lack many properties, must use device.description for matching
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

    let mut cmd = Command::new("cargo");
    cmd.args([
        "run",
        "--",
        "--config",
        config_path.to_str().unwrap(),
        "--server",
        &server.socket_path(),
        "--verbose",
    ])
    .env("RUST_LOG", "debug");

    eprintln!("TEST: Running cargo with args: {cmd:?}");

    let mut autopulsed =
        OutputCapturer::spawn(cmd).expect("Failed to spawn autopulsed");

    autopulsed.expect_string("Connected to PulseAudio server");
    autopulsed.expect_string("Found sink");
    autopulsed.expect_string("Found source");

    // Device numbers vary between test runs
    autopulsed.expect_regex(r"Sink #\d+ is detected as 'test_device_1'");
    autopulsed.expect_regex(r"Sink #\d+ is detected as 'test_device_2'");
    autopulsed.expect_regex(r"Source #\d+ is detected as 'test_monitor_1'");
    autopulsed.expect_regex(r"Source #\d+ is detected as 'test_monitor_2'");

    autopulsed
        .expect_string("Successfully set default sink")
        .expect_string("Successfully set default source");

    eprintln!("TEST: Killing autopulsed process");
    autopulsed.kill().ok();
    eprintln!("TEST: Test completed successfully");
}

#[test]
fn test_connection_to_nonexistent_server() {
    use helpers::OutputCapturer;

    // /dev/null as server guarantees connection failure
    let mut cmd = Command::new("cargo");
    cmd.args(["run", "--", "--server", "/dev/null"])
        .env("RUST_LOG", "info");

    let mut autopulsed =
        OutputCapturer::spawn(cmd).expect("Failed to spawn autopulsed");

    autopulsed.assert_exit_failure(Duration::from_secs(2));
    autopulsed.expect_string("Failed to connect to PulseAudio");
}

#[test]
fn test_server_option_overrides_env() {
    use helpers::OutputCapturer;

    let server = IsolatedPulseServer::start()
        .expect("Failed to start isolated PulseAudio server");

    // PULSE_SERVER env should be overridden by --server option
    let mut cmd = Command::new("cargo");
    cmd.args(["run", "--", "--server", &server.socket_path(), "--verbose"])
        .env("PULSE_SERVER", "/dev/null")
        .env("RUST_LOG", "debug");

    eprintln!("TEST: Running cargo with PULSE_SERVER=/dev/null");

    let mut autopulsed =
        OutputCapturer::spawn(cmd).expect("Failed to spawn autopulsed");

    autopulsed.expect_string_timeout(
        "Connected to PulseAudio server",
        Duration::from_secs(5),
    );

    eprintln!("TEST: Killing autopulsed process");
    autopulsed.kill().ok();
    eprintln!("TEST: autopulsed process terminated");
}

#[test]
fn test_remap_functionality() {
    use helpers::OutputCapturer;

    let server = IsolatedPulseServer::start()
        .expect("Failed to start isolated PulseAudio server");

    // Config with remap entries
    let config_content = r#"
sinks:
  master_sink:
    priority: 1
    detect:
      device.description: "TestSink1"
  remapped_sink:
    priority: 2
    remap:
      master: "master_sink"
      device_name: "remapped_test_sink"
      device_properties:
        device.description: "Remapped Test Sink"
      channels: 2
      channel_map: "front-left,front-right"

sources:
  master_source:
    priority: 1
    detect:
      device.description: "Monitor of TestSink2"
  remapped_source:
    priority: 2
    remap:
      master: "master_source"
      device_name: "remapped_test_source"
      device_properties:
        device.description: "Remapped Test Source"
"#;

    let config_path = server.temp_dir.path().join("test_remap_config.yml");
    std::fs::write(&config_path, config_content)
        .expect("Failed to write test config");

    let mut cmd = Command::new("cargo");
    cmd.args([
        "run",
        "--",
        "--config",
        config_path.to_str().unwrap(),
        "--server",
        &server.socket_path(),
        "--verbose",
    ])
    .env("RUST_LOG", "debug");

    eprintln!("TEST: Running autopulsed with remap config");

    let mut autopulsed =
        OutputCapturer::spawn(cmd).expect("Failed to spawn autopulsed");

    // Wait for connection
    autopulsed.expect_string("Connected to PulseAudio server");

    // Verify master devices are detected
    autopulsed.expect_regex(r"Sink #\d+ is detected as 'master_sink'");
    autopulsed.expect_regex(r"Source #\d+ is detected as 'master_source'");

    // Verify remap modules are loaded
    autopulsed.expect_string("Loading sink remap module for 'remapped_sink'");
    autopulsed.expect_regex(
        r"Successfully loaded sink remap module #\d+ for 'remapped_sink'",
    );
    autopulsed
        .expect_string("Loading source remap module for 'remapped_source'");
    autopulsed.expect_regex(
        r"Successfully loaded source remap module #\d+ for 'remapped_source'",
    );

    // Verify remap devices are actually created
    autopulsed.expect_string("Found sink #");
    autopulsed.expect_string(
        "name = remapped_test_sink, description = Remapped Test Sink",
    );
    autopulsed.expect_string("Found source #");
    autopulsed.expect_string(
        "name = remapped_test_source, description = Remapped Test Source",
    );

    eprintln!("TEST: Remap modules loaded and devices created successfully");

    // TODO: In a real test, we would:
    // 1. Remove the master device (e.g., unload module-null-sink)
    // 2. Verify remap modules are unloaded
    // But this requires more complex PulseAudio manipulation

    eprintln!("TEST: Killing autopulsed process");
    autopulsed.kill().ok();
    eprintln!("TEST: Remap test completed successfully");
}

#[test]
fn test_circular_reference_detection() {
    use helpers::OutputCapturer;

    // Config with circular references
    let config_content = r#"
sinks:
  sink_a:
    priority: 1
    remap:
      master: "sink_b"
      device_name: "circular_a"
  sink_b:
    priority: 2
    remap:
      master: "sink_c"
      device_name: "circular_b"
  sink_c:
    priority: 3
    remap:
      master: "sink_a"
      device_name: "circular_c"
"#;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_path = temp_dir.path().join("circular_config.yml");
    std::fs::write(&config_path, config_content)
        .expect("Failed to write test config");

    let mut cmd = Command::new("cargo");
    cmd.args(["run", "--", "--config", config_path.to_str().unwrap()])
        .env("RUST_LOG", "info");

    eprintln!("TEST: Running autopulsed with circular reference config");

    let mut autopulsed =
        OutputCapturer::spawn(cmd).expect("Failed to spawn autopulsed");

    // Should fail immediately with circular reference error
    autopulsed.assert_exit_failure(Duration::from_secs(2));
    // Match any of the three valid cycle patterns
    autopulsed.expect_regex(
        r"Circular reference detected in sinks: (sink_a -> sink_b -> sink_c -> sink_a|sink_b -> sink_c -> sink_a -> sink_b|sink_c -> sink_a -> sink_b -> sink_c)"
    );

    eprintln!(
        "TEST: Circular reference detection test completed successfully"
    );
}

#[test]
fn test_remap_with_nonexistent_master() {
    use helpers::OutputCapturer;

    let server = IsolatedPulseServer::start()
        .expect("Failed to start isolated PulseAudio server");

    // Config with remap referencing non-existent master
    let config_content = r#"
sinks:
  remapped_sink:
    priority: 1
    remap:
      master: "nonexistent_master"
      device_name: "orphan_remap"
"#;

    let config_path = server.temp_dir.path().join("orphan_remap_config.yml");
    std::fs::write(&config_path, config_content)
        .expect("Failed to write test config");

    let mut cmd = Command::new("cargo");
    cmd.args([
        "run",
        "--",
        "--config",
        config_path.to_str().unwrap(),
        "--server",
        &server.socket_path(),
        "--verbose",
    ])
    .env("RUST_LOG", "debug");

    eprintln!("TEST: Running autopulsed with non-existent master reference");

    let mut autopulsed =
        OutputCapturer::spawn(cmd).expect("Failed to spawn autopulsed");

    // Should start successfully (non-existent master is not an error)
    autopulsed.expect_string("Connected to PulseAudio server");
    autopulsed.expect_string("Loaded config from");

    // Should NOT try to load remap module since master doesn't exist
    autopulsed
        .expect_no_string("Loading sink remap module", Duration::from_secs(2));

    eprintln!("TEST: Killing autopulsed process");
    autopulsed.kill().ok();
    eprintln!("TEST: Non-existent master test completed successfully");
}
