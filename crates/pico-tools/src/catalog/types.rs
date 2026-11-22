//! Core data types and traits for Pico's massive operating system tool ecosystem.
//! Provides unified metadata, latency, reversibility, and permission classifications.

use async_trait::async_trait;
use pico_core::error::Result;
use pico_core::traits::{PermissionTier, Tool};
use serde::{Deserialize, Serialize};

/// Target operating system for an ecosystem tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetOs {
    Windows,
    Linux,
    Universal,
}

impl TargetOs {
    pub fn as_str(&self) -> &'static str {
        match self {
            TargetOs::Windows => "windows",
            TargetOs::Linux => "linux",
            TargetOs::Universal => "universal",
        }
    }
}

/// Expected execution latency class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LatencyClass {
    SubMillisecond,
    FastSync,
    AsyncLongRunning,
}

impl LatencyClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            LatencyClass::SubMillisecond => "sub_millisecond",
            LatencyClass::FastSync => "fast_sync",
            LatencyClass::AsyncLongRunning => "async_long_running",
        }
    }
}

/// Operational reversibility classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reversibility {
    Reversible,
    NonReversible,
    Idempotent,
}

impl Reversibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Reversibility::Reversible => "reversible",
            Reversibility::NonReversible => "non_reversible",
            Reversibility::Idempotent => "idempotent",
        }
    }
}

/// Parameter definition for an ecosystem tool action.
#[derive(Debug, Clone, Copy)]
pub struct ParamDef {
    pub name: &'static str,
    pub param_type: &'static str,
    pub description: &'static str,
    pub required: bool,
}

/// Raw definition specification for a tool action.
#[derive(Debug, Clone, Copy)]
pub struct ActionDef {
    pub subaction: &'static str,
    pub description: &'static str,
    pub tier: PermissionTier,
    pub latency: LatencyClass,
    pub reversibility: Reversibility,
    pub params: &'static [ParamDef],
    pub adapter_cmd: &'static str,
    pub adapter_args: &'static str,
}

/// Category definition containing domain actions.
#[derive(Debug, Clone, Copy)]
pub struct CategoryDef {
    pub name: &'static str,
    pub domain_label: &'static str,
    pub actions: &'static [ActionDef],
}

/// Complete metadata record for an ecosystem tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub canonical_id: String,
    pub domain_category: String,
    pub target_os: TargetOs,
    pub description: String,
    pub parameters_schema: serde_json::Value,
    pub permission_tier: PermissionTier,
    pub latency_class: LatencyClass,
    pub reversibility: Reversibility,
    pub adapter_cmd: String,
    pub adapter_args: String,
}

/// Dynamic executable tool in Pico's massive operating system ecosystem.
pub struct EcosystemTool {
    meta: ToolMetadata,
}

impl EcosystemTool {
    pub fn new(meta: ToolMetadata) -> Self {
        Self { meta }
    }

    pub fn metadata(&self) -> &ToolMetadata {
        &self.meta
    }

    pub fn target_os(&self) -> TargetOs {
        self.meta.target_os
    }

    pub fn domain_category(&self) -> &str {
        &self.meta.domain_category
    }

    pub fn latency_class(&self) -> LatencyClass {
        self.meta.latency_class
    }

    pub fn reversibility(&self) -> Reversibility {
        self.meta.reversibility
    }
}

#[async_trait]
impl Tool for EcosystemTool {
    fn name(&self) -> &str {
        &self.meta.canonical_id
    }

    fn description(&self) -> &str {
        &self.meta.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.meta.parameters_schema.clone()
    }

    fn permission_tier(&self) -> PermissionTier {
        self.meta.permission_tier
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<serde_json::Value> {
        crate::catalog::adapter::execute_tool_adapter(&self.meta, arguments).await
    }
}
