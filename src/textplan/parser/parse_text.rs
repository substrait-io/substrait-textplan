// SPDX-License-Identifier: Apache-2.0

//! Parse a textplan from a string or file using ANTLR4.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use crate::textplan::common::error::TextPlanError;
use crate::textplan::common::parse_result::ParseResult;
use crate::textplan::parser::antlr::substraitplanparser::PlanContext;
use crate::textplan::parser::error_listener::ErrorListener;
use crate::textplan::parser::grammar;
use crate::textplan::parser::visitors::PlanVisitor;
use crate::textplan::printer::plan_printer::{PlanPrinter, TextPlanFormat};
use crate::textplan::symbol_table::SymbolTable;

/// Loads a textplan from a file.
///
/// # Arguments
///
/// * `filename` - The path to the file to load.
///
/// # Returns
///
/// An optional string containing the file's contents.
pub fn load_text_file(filename: &str) -> Option<String> {
    let path = Path::new(filename);
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return None,
    };

    let mut content = String::new();
    if file.read_to_string(&mut content).is_err() {
        return None;
    }

    Some(content)
}

/// Loads a textplan from a string.
///
/// # Arguments
///
/// * `text` - The text to load.
///
/// # Returns
///
/// The text as a string.
pub fn load_text_string(text: &str) -> String {
    text.to_string()
}

/// Parses a textplan from a string using ANTLR4.
///
/// # Arguments
///
/// * `text` - The text to parse.
///
/// # Returns
///
/// The parse result.
pub fn parse_stream(text: &str) -> ParseResult {
    // If the text is empty, return an empty result
    if text.trim().is_empty() {
        return ParseResult::new(SymbolTable::new(), Vec::new(), Vec::new());
    }

    // Try to parse the text using ANTLR
    // The parse_string function now handles the visitor processing internally
    match grammar::parse_string(text) {
        Ok(grammar_result) => {
            // Get any errors from the error listener
            let error_messages = if grammar_result.error_listener.has_errors() {
                grammar_result
                    .error_listener
                    .format_errors()
                    .into_iter()
                    .map(|msg| format!("ANTLR parsing error: {}", msg))
                    .collect()
            } else {
                Vec::new()
            };

            // Return the parse result with the symbol table and any errors
            ParseResult::new(grammar_result.symbol_table, error_messages, Vec::new())
        }
        Err(err) => {
            // If parsing fails, return an error result
            ParseResult::new(SymbolTable::new(), vec![err], Vec::new())
        }
    }
}

/// Serializes a symbol table back to a textplan string.
///
/// # Arguments
///
/// * `symbol_table` - The symbol table to serialize.
/// * `format` - The format to use for the output.
///
/// # Returns
///
/// The serialized textplan as a string, or an error.
pub fn serialize_to_text(
    symbol_table: &SymbolTable,
    format: TextPlanFormat,
) -> Result<String, TextPlanError> {
    // Create a plan printer with the specified format
    let mut printer = PlanPrinter::new(format);

    // Use the printer to convert the symbol table to a textplan
    printer.print_plan(symbol_table)
}

/// Phase 1: Applies the TypeVisitor to the parse tree
///
/// This function processes type information in the parse tree using the TypeVisitor.
fn apply_type_visitor(
    plan_ctx: &PlanContext,
    symbol_table: SymbolTable,
    error_listener: Arc<ErrorListener>,
) -> SymbolTable {
    println!("Applying TypeVisitor");

    // Create a TypeVisitor with the current symbol table
    let mut type_visitor =
        crate::textplan::parser::visitors::TypeVisitor::new(symbol_table, error_listener);

    // Apply the visitor to the parse tree using our helper function
    crate::textplan::parser::visitors::visit_plan(&mut type_visitor, plan_ctx);

    // Return the updated symbol table
    type_visitor.symbol_table()
}

/// Phase 2: Applies the MainPlanVisitor to the parse tree
///
/// This function processes plan structures in the parse tree using the MainPlanVisitor.
fn apply_plan_visitor(
    plan_ctx: &PlanContext,
    symbol_table: SymbolTable,
    error_listener: Arc<ErrorListener>,
) -> SymbolTable {
    println!("Applying MainPlanVisitor");

    // Create a MainPlanVisitor with the symbol table from the previous phase
    let mut plan_visitor =
        crate::textplan::parser::visitors::MainPlanVisitor::new(symbol_table, error_listener);

    // Apply the visitor to the parse tree using our helper function
    crate::textplan::parser::visitors::visit_plan(&mut plan_visitor, plan_ctx);

    // Return the updated symbol table
    plan_visitor.symbol_table()
}

/// Phase 3: Applies the PipelineVisitor to the parse tree
///
/// This function processes pipeline structures in the parse tree using the PipelineVisitor.
fn apply_pipeline_visitor(
    plan_ctx: &PlanContext,
    symbol_table: SymbolTable,
    error_listener: Arc<ErrorListener>,
) -> SymbolTable {
    println!("Applying PipelineVisitor");

    // Create a PipelineVisitor with the symbol table from the previous phase
    let mut pipeline_visitor =
        crate::textplan::parser::visitors::PipelineVisitor::new(symbol_table, error_listener);

    // Apply the visitor to the parse tree using our helper function
    crate::textplan::parser::visitors::visit_plan(&mut pipeline_visitor, plan_ctx);

    // Return the updated symbol table
    pipeline_visitor.symbol_table()
}

/// Phase 4: Applies the RelationVisitor to the parse tree
///
/// This function processes relation structures in the parse tree using the RelationVisitor.
fn apply_relation_visitor(
    plan_ctx: &PlanContext,
    symbol_table: SymbolTable,
    error_listener: Arc<ErrorListener>,
) -> SymbolTable {
    println!("Applying RelationVisitor");

    // Create a RelationVisitor with the symbol table from the previous phase
    let mut relation_visitor =
        crate::textplan::parser::visitors::RelationVisitor::new(symbol_table, error_listener);

    // Apply the visitor to the parse tree using our helper function
    crate::textplan::parser::visitors::visit_plan(&mut relation_visitor, plan_ctx);

    // Return the updated symbol table
    relation_visitor.symbol_table()
}

/// Phase 5: Applies the SubqueryRelationVisitor to the parse tree
///
/// This function processes subquery structures in the parse tree using the SubqueryRelationVisitor.
fn apply_subquery_visitor(
    plan_ctx: &PlanContext,
    symbol_table: SymbolTable,
    error_listener: Arc<ErrorListener>,
) -> SymbolTable {
    println!("Applying SubqueryRelationVisitor");

    // Create a SubqueryRelationVisitor with the symbol table from the previous phase
    let mut subquery_visitor = crate::textplan::parser::visitors::SubqueryRelationVisitor::new(
        symbol_table,
        error_listener,
    );

    // Apply the visitor to the parse tree using our helper function
    crate::textplan::parser::visitors::visit_plan(&mut subquery_visitor, plan_ctx);

    // Return the updated symbol table
    subquery_visitor.symbol_table()
}
