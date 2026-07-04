// SPDX-License-Identifier: Apache-2.0

//! The textplan module contains code for parsing and generating Substrait plans.

pub mod common;
pub mod converter;
pub mod parser;
pub mod printer;
pub mod symbol_table;

#[cfg(test)]
mod tests;

// Re-export common types
pub use common::parse_result::ParseResult;
pub use common::Location;
pub use common::ProtoLocation;
pub use common::TextLocation;
pub use printer::plan_printer::{PlanPrinter, TextPlanFormat};
pub use symbol_table::{RelationType, SymbolInfo, SymbolTable, SymbolType};
