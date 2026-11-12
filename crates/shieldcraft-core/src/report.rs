//! Per-job processing reports (reproducibility information). Only the source file *name* is
//! stored, never its full path.

use crate::engine::EngineId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessingReport {
    pub app: String,
    pub app_version: String,
    pub engine: EngineId,
    pub engine_name: String,
    pub engine_origin: String,
    pub method: String,
    pub source_name: String,
    pub input_size: [u32; 2],
    pub output_size: [u32; 2],
    pub parameters: serde_json::Value,
    pub started_utc: String,
    pub seconds: f64,
    pub execution: String,
    pub warnings: Vec<String>,
    pub output_file: Option<String>,
}

impl ProcessingReport {
    pub fn new(engine: EngineId, source_name: &str, input: (u32, u32), params: serde_json::Value) -> Self {
        let info = engine.info();
        Self {
            app: "ShieldCraft".into(),
            app_version: crate::APP_VERSION.into(),
            engine,
            engine_name: info.name.into(),
            engine_origin: info.origin.into(),
            method: info.kind.label().into(),
            source_name: source_name.into(),
            input_size: [input.0, input.1],
            output_size: [0, 0],
            parameters: params,
            started_utc: crate::util::format_utc(crate::util::unix_now()),
            seconds: 0.0,
            execution: String::new(),
            warnings: vec![],
            output_file: None,
        }
    }
}
