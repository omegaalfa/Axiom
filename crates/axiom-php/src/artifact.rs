use serde::{Deserialize, Serialize};

use crate::{STUB_PARSER_VERSION, Symbol};

pub const EMBEDDED_STUB_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddedStubArtifact {
    pub schema_version: u32,
    pub parser_version: u32,
    pub symbols: Vec<Symbol>,
}

impl EmbeddedStubArtifact {
    pub fn new(symbols: Vec<Symbol>) -> Self {
        Self {
            schema_version: EMBEDDED_STUB_SCHEMA_VERSION,
            parser_version: STUB_PARSER_VERSION,
            symbols,
        }
    }
}
