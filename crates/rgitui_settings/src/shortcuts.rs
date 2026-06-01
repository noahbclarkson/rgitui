use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CustomShortcuts {
    pub overrides: HashMap<String, String>,
}

impl Default for CustomShortcuts {
    fn default() -> Self {
        Self {
            overrides: HashMap::new(),
        }
    }
}
