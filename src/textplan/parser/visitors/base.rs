// SPDX-License-Identifier: Apache-2.0

//! Base visitor implementation.

use std::sync::Arc;

use crate::textplan::parser::error_listener::ErrorListener;
use crate::textplan::symbol_table::SymbolTable;

use super::PlanVisitor;

/// Base implementation for ANTLR-based Substrait plan visitors.
///
/// This provides common functionality for all visitor implementations.
pub struct BasePlanVisitor {
    symbol_table: SymbolTable,
    error_listener: Arc<ErrorListener>,
}

impl BasePlanVisitor {
    /// Creates a new base plan visitor.
    pub fn new(symbol_table: SymbolTable, error_listener: Arc<ErrorListener>) -> Self {
        Self {
            symbol_table,
            error_listener,
        }
    }

    /// Gets a mutable reference to the symbol table for modifications.
    pub fn symbol_table_mut(&mut self) -> &mut SymbolTable {
        &mut self.symbol_table
    }
}

impl<'input> PlanVisitor<'input> for BasePlanVisitor {
    fn error_listener(&self) -> Arc<ErrorListener> {
        self.error_listener.clone()
    }

    fn symbol_table(&self) -> SymbolTable {
        self.symbol_table.clone()
    }
}
