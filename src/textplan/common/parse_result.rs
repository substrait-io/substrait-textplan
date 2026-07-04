// SPDX-License-Identifier: Apache-2.0

//! The parse_result module provides a way to track the results of parsing a textplan.

use std::fmt;

use crate::textplan::symbol_table::SymbolTable;

/// Represents the result of parsing a textplan.
///
/// This contains the symbol table, any syntax errors, and any semantic errors.
#[derive(Debug)]
pub struct ParseResult {
    symbol_table: SymbolTable,
    syntax_errors: Vec<String>,
    semantic_errors: Vec<String>,
}

impl ParseResult {
    /// Creates a new ParseResult with the given symbol table and errors.
    pub fn new(
        symbol_table: SymbolTable,
        syntax_errors: Vec<String>,
        semantic_errors: Vec<String>,
    ) -> Self {
        Self {
            symbol_table,
            syntax_errors,
            semantic_errors,
        }
    }

    /// Returns true if the parse was successful (no errors).
    pub fn successful(&self) -> bool {
        self.syntax_errors.is_empty() && self.semantic_errors.is_empty()
    }

    /// Returns a reference to the symbol table.
    pub fn symbol_table(&self) -> &SymbolTable {
        &self.symbol_table
    }

    /// Returns a reference to the syntax errors.
    pub fn syntax_errors(&self) -> &[String] {
        &self.syntax_errors
    }

    /// Returns a reference to the semantic errors.
    pub fn semantic_errors(&self) -> &[String] {
        &self.semantic_errors
    }

    /// Returns a vector of all errors (syntax and semantic).
    pub fn all_errors(&self) -> Vec<String> {
        let mut errors = self.syntax_errors.clone();
        errors.extend_from_slice(&self.semantic_errors);
        errors
    }

    /// Adds the given errors to the syntax errors.
    pub fn add_errors(&mut self, errors: &[String]) {
        self.syntax_errors.extend_from_slice(errors);
    }
}

impl fmt::Display for ParseResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.successful() {
            write!(f, "Successful parse")
        } else {
            writeln!(f, "Parse failed with {} errors:", self.all_errors().len())?;
            for error in self.all_errors() {
                writeln!(f, "  {}", error)?;
            }
            Ok(())
        }
    }
}
