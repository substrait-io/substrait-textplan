// SPDX-License-Identifier: Apache-2.0

//! Load a JSON Substrait plan and convert it to a textplan.

use std::fs;
use std::path::Path;

use crate::proto::{self, Plan};
use crate::textplan::common::error::TextPlanError;

/// Loads a JSON Substrait plan from a string and converts it to a Plan protobuf.
///
/// # Arguments
///
/// * `json_str` - The JSON string containing the plan.
///
/// # Returns
///
/// The Plan protobuf representation of the JSON.
pub fn load_plan_from_json_str(json_str: &str) -> Result<Plan, TextPlanError> {
    proto::load_plan_from_json(json_str)
}

/// Loads a JSON Substrait plan from a file and converts it to a Plan protobuf.
///
/// # Arguments
///
/// * `file_path` - The path to the JSON file.
///
/// # Returns
///
/// The Plan protobuf representation of the JSON file.
pub fn load_from_json_file<P: AsRef<Path>>(file_path: P) -> Result<Plan, TextPlanError> {
    // Read the file
    let json_str = fs::read_to_string(&file_path).map_err(TextPlanError::IoError)?;

    // Parse the JSON
    load_plan_from_json_str(&json_str)
}
