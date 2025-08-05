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

use regex::Regex;
use std::io::{BufRead, BufReader};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// A process output capturer that allows non-consuming pattern matching
///
/// Unlike tools like `rexpect` that consume output as they match patterns,
/// OutputCapturer keeps all output in memory and allows searching for patterns
/// multiple times without consumption. This enables:
/// - Order-independent pattern matching
/// - Early test termination when all conditions are met
/// - Better debugging with full output on failure
///
/// # Example Usage
///
/// ```rust
/// use helpers::OutputCapturer;
/// use std::process::Command;
/// use std::time::Duration;
///
/// // Build a command to run
/// let mut cmd = Command::new("my_program");
/// cmd.arg("--verbose")
///    .env("RUST_LOG", "debug");
///
/// // Spawn the process with OutputCapturer
/// let mut capturer = OutputCapturer::spawn(cmd)
///     .expect("Failed to spawn process");
///
/// // Method 1: Chain multiple expectations
/// capturer
///     .expect_string("Server started")
///     .expect_string("Listening on port 8080")
///     .expect_regex(r"Connected clients: \d+");
///
/// // Method 2: Use expectations separately (no chaining required)
/// capturer.expect_string("Database connected");
/// capturer.expect_string("Cache initialized");
///
/// // Method 3: Use with custom timeout
/// capturer.expect_string_timeout("Slow operation complete", Duration::from_secs(10));
///
/// // Method 4: Use regex patterns
/// capturer.expect_regex(r"Connected clients: \d+");
/// capturer.expect_regex(r"Port: \d{4}")
///
/// // Method 5: Check early exit with failure
/// capturer.assert_exit_failure(Duration::from_secs(2));
///
/// // Clean up
/// capturer.kill().ok();
/// ```
///
/// # Pattern Matching Types
///
/// - `expect_string()` - Substring matching (case-sensitive)
/// - `expect_regex()` - Regular expression matching
/// - `expect_string_timeout()` - String matching with custom timeout
/// - `expect_regex_timeout()` - Regex matching with custom timeout
/// - `assert_exit_failure()` - Assert process exits with error
/// - `assert_exit_success()` - Assert process exits successfully
/// - `wait_for_exit()` - Wait for process exit and get status
///
/// # Key Features
///
/// 1. **Non-consuming**: Patterns can be searched multiple times
/// 2. **Order-independent**: Can find patterns regardless of order
/// 3. **Early termination**: Tests complete as soon as conditions are met
/// 4. **Debug-friendly**: Shows all collected output on failure
/// 5. **Flexible API**: Chainable methods but chaining is optional
pub struct OutputCapturer {
    child: Child,
    output: Arc<Mutex<String>>,
    reader_thread: Option<thread::JoinHandle<()>>,
}

impl OutputCapturer {
    /// Spawn a process and start collecting its output
    pub fn spawn(
        mut command: std::process::Command,
    ) -> Result<Self, std::io::Error> {
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        let mut child = command.spawn()?;
        let output = Arc::new(Mutex::new(String::new()));

        let stdout = child.stdout.take().expect("Failed to get stdout");
        let stderr = child.stderr.take().expect("Failed to get stderr");

        // Separate threads prevent blocking on either stream
        let output_clone1 = output.clone();
        let stdout_thread = thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if let Ok(line) = line {
                    println!("STDOUT: {line}"); // Aid debugging when tests fail
                    if let Ok(mut out) = output_clone1.lock() {
                        out.push_str(&line);
                        out.push('\n');
                    }
                }
            }
        });

        let output_clone2 = output.clone();
        let stderr_thread = thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                if let Ok(line) = line {
                    println!("STDERR: {line}"); // Aid debugging when tests fail
                    if let Ok(mut out) = output_clone2.lock() {
                        out.push_str(&line);
                        out.push('\n');
                    }
                }
            }
        });

        let reader_thread = thread::spawn(move || {
            stdout_thread.join().ok();
            stderr_thread.join().ok();
        });

        Ok(OutputCapturer {
            child,
            output,
            reader_thread: Some(reader_thread),
        })
    }

    /// Wait for a pattern to appear in the output (non-consuming)
    fn wait_for(
        &self,
        pattern: &str,
        timeout: Duration,
    ) -> Result<(), String> {
        let start = Instant::now();

        while start.elapsed() < timeout {
            if let Ok(out) = self.output.lock() {
                if out.contains(pattern) {
                    return Ok(());
                }
            }
            thread::sleep(Duration::from_millis(50));
        }

        if let Ok(out) = self.output.lock() {
            Err(format!(
                "Timeout waiting for '{}'. Collected output ({} bytes):\n{}",
                pattern,
                out.len(),
                out
            ))
        } else {
            Err(format!("Timeout waiting for '{pattern}'"))
        }
    }

    /// Wait for a regex pattern to appear in the output (non-consuming)
    fn wait_for_regex(
        &self,
        pattern: &str,
        timeout: Duration,
    ) -> Result<(), String> {
        let re = Regex::new(pattern)
            .map_err(|e| format!("Invalid regex '{pattern}': {e}"))?;

        let start = Instant::now();

        while start.elapsed() < timeout {
            if let Ok(out) = self.output.lock() {
                if re.is_match(&out) {
                    return Ok(());
                }
            }
            thread::sleep(Duration::from_millis(50));
        }

        if let Ok(out) = self.output.lock() {
            Err(format!(
                "Timeout waiting for regex '{}'. Collected output ({} bytes):\n{}",
                pattern,
                out.len(),
                out
            ))
        } else {
            Err(format!("Timeout waiting for regex '{pattern}'"))
        }
    }

    /// Wait for a string pattern (chainable)
    pub fn expect_string(&self, pattern: &str) -> &Self {
        self.expect_string_timeout(pattern, Duration::from_secs(5))
    }

    /// Wait for a string pattern with custom timeout (chainable)
    pub fn expect_string_timeout(
        &self,
        pattern: &str,
        timeout: Duration,
    ) -> &Self {
        match self.wait_for(pattern, timeout) {
            Ok(()) => {
                eprintln!("✓ Found: {pattern}");
                self
            }
            Err(e) => {
                panic!("Failed to find pattern '{pattern}': {e}");
            }
        }
    }

    /// Wait for a regex pattern (chainable)
    pub fn expect_regex(&self, pattern: &str) -> &Self {
        self.expect_regex_timeout(pattern, Duration::from_secs(5))
    }

    /// Wait for a regex pattern with custom timeout (chainable)
    pub fn expect_regex_timeout(
        &self,
        pattern: &str,
        timeout: Duration,
    ) -> &Self {
        match self.wait_for_regex(pattern, timeout) {
            Ok(()) => {
                eprintln!("✓ Found regex: {pattern}");
                self
            }
            Err(e) => {
                panic!("Failed to find regex '{pattern}': {e}");
            }
        }
    }

    /// Expect string to NOT appear within timeout
    pub fn expect_no_string(&self, pattern: &str, timeout: Duration) -> &Self {
        let start = Instant::now();

        while start.elapsed() < timeout {
            let output = self.output.lock().unwrap();
            if output.contains(pattern) {
                eprintln!("=== Full output ===");
                eprintln!("{}", *output);
                eprintln!("==================");
                panic!("Found unexpected pattern '{pattern}' in output");
            }
            drop(output);
            thread::sleep(Duration::from_millis(50));
        }

        eprintln!("✓ Pattern not found as expected: {pattern}");
        self
    }

    /// Expect regex to NOT match within timeout
    pub fn expect_no_regex(&self, pattern: &str, timeout: Duration) -> &Self {
        let re = regex::Regex::new(pattern)
            .map_err(|e| format!("Invalid regex '{pattern}': {e}"))
            .unwrap();

        let start = Instant::now();

        while start.elapsed() < timeout {
            let output = self.output.lock().unwrap();
            if re.is_match(&output) {
                eprintln!("=== Full output ===");
                eprintln!("{}", *output);
                eprintln!("==================");
                panic!("Found unexpected regex match '{pattern}' in output");
            }
            drop(output);
            thread::sleep(Duration::from_millis(50));
        }

        eprintln!("✓ Regex not matched as expected: {pattern}");
        self
    }

    /// Kill the process
    pub fn kill(&mut self) -> Result<(), std::io::Error> {
        self.child.kill()
    }

    /// Wait for process to exit and return exit status
    pub fn wait_for_exit(
        &mut self,
        timeout: Duration,
    ) -> Result<ExitStatus, String> {
        let start = Instant::now();

        while start.elapsed() < timeout {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    // Wait for threads to finish collecting remaining output
                    if let Some(thread) = self.reader_thread.take() {
                        let _ = thread.join();
                    }
                    return Ok(status);
                }
                Ok(None) => {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    return Err(format!(
                        "Failed to check process status: {e}"
                    ));
                }
            }
        }

        Err(format!("Process did not exit within {timeout:?}"))
    }

    /// Check if process is still running
    pub fn is_running(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(None) => true,
            _ => false,
        }
    }

    /// Assert that process exited with failure
    pub fn assert_exit_failure(&mut self, timeout: Duration) {
        match self.wait_for_exit(timeout) {
            Ok(status) => {
                if status.success() {
                    panic!("Expected process to fail, but it succeeded");
                }
                eprintln!("✓ Process exited with failure as expected");
            }
            Err(e) => {
                panic!("Failed to get exit status: {e}");
            }
        }
    }

    /// Assert that process exited with success
    pub fn assert_exit_success(&mut self, timeout: Duration) {
        match self.wait_for_exit(timeout) {
            Ok(status) => {
                if !status.success() {
                    panic!(
                        "Expected process to succeed, but it failed with status: {status:?}"
                    );
                }
                eprintln!("✓ Process exited successfully");
            }
            Err(e) => {
                panic!("Failed to get exit status: {e}");
            }
        }
    }

    /// Get a snapshot of the current output
    pub fn get_output(&self) -> String {
        self.output
            .lock()
            .map(|out| out.clone())
            .unwrap_or_default()
    }

    /// Extract captured groups from a regex pattern
    pub fn extract_regex(&self, pattern: &str) -> Option<Vec<String>> {
        let re = Regex::new(pattern).ok()?;
        let output = self.get_output();

        re.captures(&output).map(|caps| {
            caps.iter()
                .skip(1) // Skip the full match
                .filter_map(|m| m.map(|m| m.as_str().to_string()))
                .collect()
        })
    }
}

impl Drop for OutputCapturer {
    fn drop(&mut self) {
        let _ = self.child.kill();

        if let Some(thread) = self.reader_thread.take() {
            let _ = thread.join();
        }

        let _ = self.child.wait();
    }
}
