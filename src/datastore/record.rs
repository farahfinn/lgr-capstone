use serde::{Deserialize, Serialize};

/// This will be a single K,V record stored in the db file
#[derive(Debug, Serialize, Deserialize)]
pub struct Record {
    pub key: String,
    pub val: String,
}
