// SPDX-License-Identifier: Apache-2.0

//! Loads a textplan from a string and converts it to a binary protobuf.

use crate::textplan::common::error::TextPlanError;
use crate::textplan::converter::save_binary::save_to_binary;
use crate::textplan::parser::parse_text::parse_stream;

/// Loads a textplan from a string and converts it to a binary protobuf.
///
/// # Arguments
///
/// * `text` - The textplan to load.
///
/// # Returns
///
/// The binary protobuf representation of the plan.
pub fn load_from_text(text: &str) -> Result<Vec<u8>, TextPlanError> {
    let parse_result = parse_stream(text);

    if !parse_result.successful() {
        let errors = parse_result.all_errors();
        let error_msg = errors.join("\n");
        return Err(TextPlanError::ParseError(error_msg));
    }

    // Convert the symbol table to a protobuf plan
    save_to_binary(parse_result.symbol_table())
}
