// SPDX-License-Identifier: Apache-2.0

//! Save a Substrait plan to JSON format.

use std::fs;
use std::path::Path;

use crate::proto::{save_plan_to_json, Plan};
use crate::textplan::common::error::TextPlanError;
use crate::textplan::symbol_table::SymbolTable;

/// Saves a Substrait plan to JSON format.
///
/// # Arguments
///
/// * `plan` - The plan to save.
///
/// # Returns
///
/// The JSON representation of the plan.
pub fn save_to_json(plan: &Plan) -> Result<String, TextPlanError> {
    save_plan_to_json(plan)
}

/// Saves a Substrait plan to a JSON file.
///
/// # Arguments
///
/// * `plan` - The plan to save.
/// * `file_path` - The path to the JSON file.
///
/// # Returns
///
/// A result indicating success or failure.
pub fn save_to_json_file<P: AsRef<Path>>(plan: &Plan, file_path: P) -> Result<(), TextPlanError> {
    let json_str = save_to_json(plan)?;

    fs::write(&file_path, json_str).map_err(TextPlanError::IoError)
}

/// Saves a symbol table to JSON format by first converting it to a Plan.
///
/// # Arguments
///
/// * `symbol_table` - The symbol table to save.
///
/// # Returns
///
/// The JSON representation of the plan.
pub fn symbol_table_to_json(symbol_table: &SymbolTable) -> Result<String, TextPlanError> {
    // Convert the symbol table to a Plan
    let plan =
        crate::textplan::converter::save_binary::create_plan_from_symbol_table(symbol_table)?;

    // Convert the Plan to JSON
    save_to_json(&plan)
}

/// Converts a textplan to JSON format.
///
/// # Arguments
///
/// * `text` - The textplan to convert.
///
/// # Returns
///
/// The JSON representation of the plan.
pub fn save_to_json_from_text(text: &str) -> Result<String, TextPlanError> {
    // Parse the textplan to a symbol table
    let parse_result = crate::textplan::parser::parse_text::parse_stream(text);

    if !parse_result.successful() {
        let errors = parse_result.all_errors();
        let error_msg = errors.join("\n");
        return Err(TextPlanError::ParseError(error_msg));
    }

    // Convert the symbol table to JSON
    symbol_table_to_json(parse_result.symbol_table())
}
