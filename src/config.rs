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

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemapConfig {
    // Required parameter
    pub master: String,

    // Device configuration (common for sink/source)
    pub device_name: Option<String>,
    pub device_properties: Option<HashMap<String, String>>,

    // Audio format
    pub format: Option<String>,
    pub rate: Option<u32>,
    pub channels: Option<u32>,

    // Channel mapping
    pub channel_map: Option<String>,
    pub master_channel_map: Option<String>,

    // Other options
    pub resample_method: Option<String>,
    pub remix: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceMatchConfig {
    Detect(HashMap<String, String>),
    Remap(RemapConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub priority: Option<u32>,
    #[serde(flatten)]
    pub match_config: DeviceMatchConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub sinks: HashMap<String, DeviceConfig>,
    #[serde(default)]
    pub sources: HashMap<String, DeviceConfig>,
}

impl Config {
    /// Validate the configuration for circular references in remap chains
    pub fn validate(&self) -> Result<(), String> {
        Self::validate_remap_references(&self.sinks, "sinks")?;
        Self::validate_remap_references(&self.sources, "sources")?;
        Ok(())
    }

    fn validate_remap_references(
        devices: &HashMap<String, DeviceConfig>,
        device_type: &str,
    ) -> Result<(), String> {
        for (name, config) in devices {
            if let DeviceMatchConfig::Remap(_) = &config.match_config {
                // Use iterative approach to detect cycles
                let mut visited = HashSet::new();
                let mut current = name.as_str();
                let mut path = vec![current];

                loop {
                    // Check if we've seen this device before (cycle detected)
                    if !visited.insert(current.to_string()) {
                        // Find where the cycle starts
                        let cycle_start =
                            path.iter().position(|&n| n == current).unwrap();
                        let cycle_path: Vec<_> = path[cycle_start..]
                            .iter()
                            .map(|s| s.to_string())
                            .collect();

                        return Err(format!(
                            "Circular reference detected in {}: {}",
                            device_type,
                            cycle_path.join(" -> ")
                        ));
                    }

                    // Get the device configuration
                    let device = match devices.get(current) {
                        Some(d) => d,
                        None => break, // Referenced device doesn't exist, not a cycle
                    };

                    // Check if this device has a remap master
                    match &device.match_config {
                        DeviceMatchConfig::Remap(remap) => {
                            current = &remap.master;
                            path.push(current);
                        }
                        DeviceMatchConfig::Detect(_) => break, // End of chain
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circular_reference_detection() {
        let mut config = Config::default();

        // Create a circular reference: a -> b -> c -> a
        config.sinks.insert(
            "a".to_string(),
            DeviceConfig {
                priority: Some(1),
                match_config: DeviceMatchConfig::Remap(RemapConfig {
                    master: "b".to_string(),
                    device_name: None,
                    device_properties: None,
                    format: None,
                    rate: None,
                    channels: None,
                    channel_map: None,
                    master_channel_map: None,
                    resample_method: None,
                    remix: None,
                }),
            },
        );

        config.sinks.insert(
            "b".to_string(),
            DeviceConfig {
                priority: Some(2),
                match_config: DeviceMatchConfig::Remap(RemapConfig {
                    master: "c".to_string(),
                    device_name: None,
                    device_properties: None,
                    format: None,
                    rate: None,
                    channels: None,
                    channel_map: None,
                    master_channel_map: None,
                    resample_method: None,
                    remix: None,
                }),
            },
        );

        config.sinks.insert(
            "c".to_string(),
            DeviceConfig {
                priority: Some(3),
                match_config: DeviceMatchConfig::Remap(RemapConfig {
                    master: "a".to_string(),
                    device_name: None,
                    device_properties: None,
                    format: None,
                    rate: None,
                    channels: None,
                    channel_map: None,
                    master_channel_map: None,
                    resample_method: None,
                    remix: None,
                }),
            },
        );

        let result = config.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("Circular reference detected"),
            "Error message: {err}"
        );
        // The cycle could be detected starting from any device in the cycle
        // and should show the complete cycle back to the starting point
        assert!(
            err.contains("a -> b -> c -> a")
                || err.contains("b -> c -> a -> b")
                || err.contains("c -> a -> b -> c"),
            "Error message: {err}"
        );
    }

    #[test]
    fn test_no_circular_reference() {
        let mut config = Config::default();

        // Create a valid chain: a -> b -> c (where c is a detect device)
        config.sinks.insert(
            "a".to_string(),
            DeviceConfig {
                priority: Some(1),
                match_config: DeviceMatchConfig::Remap(RemapConfig {
                    master: "b".to_string(),
                    device_name: None,
                    device_properties: None,
                    format: None,
                    rate: None,
                    channels: None,
                    channel_map: None,
                    master_channel_map: None,
                    resample_method: None,
                    remix: None,
                }),
            },
        );

        config.sinks.insert(
            "b".to_string(),
            DeviceConfig {
                priority: Some(2),
                match_config: DeviceMatchConfig::Remap(RemapConfig {
                    master: "c".to_string(),
                    device_name: None,
                    device_properties: None,
                    format: None,
                    rate: None,
                    channels: None,
                    channel_map: None,
                    master_channel_map: None,
                    resample_method: None,
                    remix: None,
                }),
            },
        );

        config.sinks.insert(
            "c".to_string(),
            DeviceConfig {
                priority: Some(3),
                match_config: DeviceMatchConfig::Detect(HashMap::new()),
            },
        );

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_self_reference() {
        let mut config = Config::default();

        // Create a self-reference: a -> a
        config.sources.insert(
            "a".to_string(),
            DeviceConfig {
                priority: Some(1),
                match_config: DeviceMatchConfig::Remap(RemapConfig {
                    master: "a".to_string(),
                    device_name: None,
                    device_properties: None,
                    format: None,
                    rate: None,
                    channels: None,
                    channel_map: None,
                    master_channel_map: None,
                    resample_method: None,
                    remix: None,
                }),
            },
        );

        let result = config.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Circular reference detected"));
        assert!(err.contains("sources"));
    }

    #[test]
    fn test_reference_to_nonexistent_device() {
        let mut config = Config::default();

        // Create a reference to a non-existent device
        config.sinks.insert(
            "a".to_string(),
            DeviceConfig {
                priority: Some(1),
                match_config: DeviceMatchConfig::Remap(RemapConfig {
                    master: "nonexistent".to_string(),
                    device_name: None,
                    device_properties: None,
                    format: None,
                    rate: None,
                    channels: None,
                    channel_map: None,
                    master_channel_map: None,
                    resample_method: None,
                    remix: None,
                }),
            },
        );

        // This should be valid - referencing a non-existent device is not a circular reference
        assert!(config.validate().is_ok());
    }
}
