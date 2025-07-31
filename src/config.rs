use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub priority: Option<u32>,
    pub detect: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub sinks: HashMap<String, DeviceConfig>,
    #[serde(default)]
    pub sources: HashMap<String, DeviceConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sinks: HashMap::new(),
            sources: HashMap::new(),
        }
    }
}
