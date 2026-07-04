// SPDX-License-Identifier: Apache-2.0

//! This module provides utilities for working with Substrait plans using the substrait crate.

use crate::textplan::common::error::TextPlanError;
use prost::Message;

// Define type aliases to make migration easier and code cleaner
pub type Plan = ::substrait::proto::Plan;
pub type Rel = ::substrait::proto::Rel;
pub type PlanRel = ::substrait::proto::PlanRel;
pub type RelCommon = ::substrait::proto::RelCommon;
pub type RelType = ::substrait::proto::rel::RelType;

/// Load a binary protobuf into a Plan
pub fn load_plan_from_binary(bytes: &[u8]) -> Result<Plan, TextPlanError> {
    Plan::decode(bytes)
        .map_err(|e| TextPlanError::ProtobufError(format!("Failed to decode Plan: {}", e)))
}

/// Save a Plan to binary protobuf format
pub fn save_plan_to_binary(plan: &Plan) -> Result<Vec<u8>, TextPlanError> {
    let mut buf = Vec::new();

    plan.encode(&mut buf)
        .map_err(|e| TextPlanError::ProtobufError(format!("Failed to encode Plan: {}", e)))?;

    Ok(buf)
}

/// Load a Plan from JSON string using standard Protobuf JSON format.
/// With the serde feature enabled on the substrait crate, this should handle
/// the standard Protobuf JSON format correctly.
pub fn load_plan_from_json(json_str: &str) -> Result<Plan, TextPlanError> {
    // Skip an initial leading comment line that starts with #
    let usable_json = if json_str.starts_with('#') {
        match json_str.find('\n') {
            Some(idx) => &json_str[idx + 1..],
            None => json_str,
        }
    } else {
        json_str
    };

    // Use serde_json directly to parse the JSON
    match serde_json::from_str::<Plan>(usable_json) {
        Ok(plan) => Ok(plan),
        Err(err) => {
            // If it fails, try to get more information about the failure
            match serde_json::from_str::<serde_json::Value>(usable_json) {
                Ok(json_value) => {
                    // Log the structure for debugging
                    log::debug!(
                        "JSON structure: {}",
                        serde_json::to_string_pretty(&json_value).unwrap_or_default()
                    );

                    if let serde_json::Value::Object(map) = &json_value {
                        let available_fields = map
                            .keys()
                            .map(|k| k.to_string())
                            .collect::<Vec<_>>()
                            .join(", ");

                        Err(TextPlanError::ProtobufError(format!(
                            "Failed to deserialize JSON to Substrait Plan. Error: {}. Available fields: {}",
                            err, available_fields
                        )))
                    } else {
                        Err(TextPlanError::ProtobufError(format!(
                            "Failed to deserialize JSON to Substrait Plan. Error: {}.",
                            err
                        )))
                    }
                }
                Err(parse_err) => Err(TextPlanError::ProtobufError(format!(
                    "Invalid JSON syntax: {}",
                    parse_err
                ))),
            }
        }
    }
}

/// Save a Plan to JSON format using the standard Protobuf JSON format
pub fn save_plan_to_json(plan: &Plan) -> Result<String, TextPlanError> {
    // Using serde_json to convert to standard Protobuf JSON format
    serde_json::to_string(plan).map_err(|e| {
        TextPlanError::ProtobufError(format!("Failed to serialize Plan to JSON: {}", e))
    })
}

// TODO: Move this to a more appropriate location.
/// Get the relation type as a string
pub fn relation_type_to_string(rel: &Rel) -> &'static str {
    use ::substrait::proto::rel::RelType;

    match &rel.rel_type {
        Some(RelType::Read(_)) => "read",
        Some(RelType::Filter(_)) => "filter",
        Some(RelType::Project(_)) => "project",
        Some(RelType::Join(_)) => "join",
        Some(RelType::Aggregate(_)) => "aggregate",
        Some(RelType::Sort(_)) => "sort",
        Some(RelType::Cross(_)) => "cross",
        Some(RelType::Fetch(_)) => "fetch",
        Some(RelType::Set(_)) => "set",
        Some(RelType::HashJoin(_)) => "hash_join",
        Some(RelType::MergeJoin(_)) => "merge_join",
        Some(RelType::NestedLoopJoin(_)) => "nested_loop_join",
        None => "unknown",
        Some(RelType::ExtensionSingle(_)) => "extension_single",
        Some(RelType::ExtensionMulti(_)) => "extension_multi",
        Some(RelType::ExtensionLeaf(_)) => "extension_leaf",
        Some(RelType::Reference(_)) => "reference",
        Some(RelType::Write(_)) => "write",
        Some(RelType::Ddl(_)) => "ddl",
        Some(RelType::Window(_)) => "window",
        Some(RelType::Exchange(_)) => "exchange",
        Some(RelType::Expand(_)) => "expand",
        Some(RelType::Update(_)) => "update",
    }
}
