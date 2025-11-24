// SPDX-License-Identifier: Apache-2.0

//! Visitor implementations for processing ANTLR parse trees.
//!
//! This module contains visitor implementations that process
//! the ANTLR parse tree and build a symbol table following the
//! multiphase approach used in the C++ implementation.

use std::sync::Arc;

use antlr_rust::token::{GenericToken, Token};

use crate::textplan::common::text_location::TextLocation;
use crate::textplan::parser::antlr::substraitplanparservisitor::SubstraitPlanParserVisitor;
use crate::textplan::parser::error_listener::ErrorListener;
use crate::textplan::symbol_table::SymbolTable;

// Module declarations
mod base;
mod main_visitor;
mod pipeline_visitor;
mod relation_visitor;
mod subquery_visitor;
mod type_visitor;

// Re-exports
pub use base::BasePlanVisitor;
pub use main_visitor::MainPlanVisitor;
pub use pipeline_visitor::PipelineVisitor;
pub use relation_visitor::RelationVisitor;
pub use subquery_visitor::SubqueryRelationVisitor;
pub use type_visitor::TypeVisitor;

/// Helper function to convert ANTLR token to TextLocation
pub fn token_to_location<'a>(
    token: &impl std::ops::Deref<Target = GenericToken<std::borrow::Cow<'a, str>>>,
) -> TextLocation {
    // Convert token position to an absolute position
    // Use both line and column to create a unique position
    // Position = (line * 10000) + column to ensure different lines have different positions
    let position = (token.line as i32 * 10000) + (token.column as i32);
    let length = token.get_text().len() as i32;
    TextLocation::new(position, length)
}

/// Helper function to extract string content by removing quotes.
/// Removes leading and trailing quotation marks from a string.
pub fn extract_from_string(s: &str) -> String {
    if s.len() < 2 {
        return s.to_string();
    }

    let mut result = s.to_string();

    // Remove trailing quote if present
    if result.ends_with('"') {
        result.pop();
    }

    // Remove leading quote if present
    if result.starts_with('"') {
        result.remove(0);
    }

    result
}

/// Helper function to safely apply a visitor to a parse tree node.
///
/// This function handles the common pattern of applying a visitor to a parse tree node,
/// properly managing the lifetimes and ownership.
pub fn visit_parse_tree<'input, V>(visitor: &mut V, context: &dyn antlr_rust::tree::Visitable<V>)
where
    V: SubstraitPlanParserVisitor<'input>,
{
    // Call the accept method on the context to apply the visitor
    context.accept(visitor);
}

/// Helper function to safely apply a visitor to a plan context.
///
/// This specializes the visit_parse_tree function for the plan rule.
pub fn visit_plan<'input, V>(
    visitor: &mut V,
    context: &crate::textplan::parser::antlr::substraitplanparser::PlanContext<'input>,
) where
    V: SubstraitPlanParserVisitor<'input>,
{
    println!("Visiting plan node");

    // Use antlr_rust::tree::Visitable trait to access the accept method
    use antlr_rust::tree::Visitable;
    context.accept(visitor);
}

/// Pre-scans the parse tree to mark all subqueries before building expressions.
///
/// This function creates a specialized visitor that only looks for SUBQUERY keywords
/// and marks those relations, without building full expressions. This ensures that
/// parent_query_info is set before expressions are built, enabling proper outer
/// reference detection.
pub fn prescan_subqueries<'input>(
    relation_visitor: &mut RelationVisitor<'input>,
    context: &crate::textplan::parser::antlr::substraitplanparser::PlanContext<'input>,
) {
    // Set a flag to indicate we're in prescan mode
    relation_visitor.set_prescan_mode(true);

    // Visit the tree to find and mark subqueries
    use antlr_rust::tree::Visitable;
    context.accept(relation_visitor);

    // Clear prescan mode
    relation_visitor.set_prescan_mode(false);

    println!("  Prescan complete - subqueries marked");
}

/// Base trait for all ANTLR-based Substrait plan visitors.
pub trait PlanVisitor<'input> {
    /// Gets the error listener for this visitor.
    fn error_listener(&self) -> Arc<ErrorListener>;

    /// Gets the symbol table for this visitor.
    fn symbol_table(&self) -> SymbolTable;
}
