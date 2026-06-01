use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CustomShortcuts {
    pub overrides: HashMap<String, String>,
}
