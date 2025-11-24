// SPDX-License-Identifier: Apache-2.0

//! Subquery visitor for processing subquery relations and outer references.

use std::sync::Arc;

use antlr_rust::parser_rule_context::ParserRuleContext;
use antlr_rust::rule_context::RuleContext;
use antlr_rust::token::{GenericToken, Token};
use antlr_rust::tree::{ParseTree, ParseTreeVisitor};
use antlr_rust::TidExt;

use crate::textplan::common::structured_symbol_data::RelationData;
use crate::textplan::parser::antlr::substraitplanparser::*;
use crate::textplan::parser::antlr::substraitplanparservisitor::SubstraitPlanParserVisitor;
use crate::textplan::parser::error_listener::ErrorListener;
use crate::textplan::symbol_table::{SymbolInfo, SymbolTable, SymbolType};

use super::{token_to_location, PlanVisitor};

/// The SubqueryRelationVisitor processes subquery relations and fixes outer references.
///
/// This visitor is the fifth phase in the multiphase parsing approach.
pub struct SubqueryRelationVisitor<'input> {
    symbol_table: SymbolTable,
    error_listener: Arc<ErrorListener>,
    current_relation_scope: Option<Arc<SymbolInfo>>,
    // Stores (field_index, steps_out) for each column expression in visit order
    expression_field_info: std::cell::RefCell<Vec<(usize, usize)>>,
    // Tracks the next index to consume from expression_field_info
    expression_field_info_index: std::cell::RefCell<usize>,
    _phantom: std::marker::PhantomData<&'input ()>,
}

impl<'input> SubqueryRelationVisitor<'input> {
    /// Creates a new SubqueryRelationVisitor.
    pub fn new(symbol_table: SymbolTable, error_listener: Arc<ErrorListener>) -> Self {
        // Note: We don't populate sub_query_pipelines here because parent_query_index
        // is not set yet. It will be set during the visit, and then we'll populate
        // sub_query_pipelines after the visit is complete.

        Self {
            symbol_table,
            error_listener,
            current_relation_scope: None,
            expression_field_info: std::cell::RefCell::new(Vec::new()),
            expression_field_info_index: std::cell::RefCell::new(0),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Call this after visiting to populate sub_query_pipelines now that parent_query_index is set.
    pub fn finalize(&mut self) {
        // Populate sub_query_pipelines using parent_query_location (not during visit)
        // This is more reliable because it uses the actual terminus relations
        Self::populate_subquery_pipelines(&mut self.symbol_table);
    }

    /// Populates sub_query_pipelines for all relations by finding subquery relations
    /// and adding them to their parent relations.
    fn populate_subquery_pipelines(symbol_table: &mut SymbolTable) {
        // Find all subquery relations (those with parent_query_index >= 0)
        let mut subqueries: Vec<Arc<SymbolInfo>> = Vec::new();
        for symbol in symbol_table.symbols() {
            if symbol.symbol_type() != SymbolType::Relation {
                continue;
            }

            println!(
                "DEBUG SUBQUERY: Checking relation '{}': parent_query_index={}",
                symbol.name(),
                symbol.parent_query_index()
            );

            // Check if this relation or its pipeline_start has parent_query_index set
            let is_subquery = if symbol.parent_query_index() >= 0 {
                println!(
                    "DEBUG SUBQUERY:   -> '{}' is a subquery (parent_query_index={})",
                    symbol.name(),
                    symbol.parent_query_index()
                );
                true
            } else if let Some(blob_lock) = &symbol.blob {
                if let Ok(blob_data) = blob_lock.lock() {
                    if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                        if let Some(pipeline_start) = &relation_data.pipeline_start {
                            pipeline_start.parent_query_index() >= 0
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

            if is_subquery {
                // Check if this relation is the terminus.
                // A terminus is a relation that is NOT anyone's continuing_pipeline.
                // In other words, nothing feeds into it - it's the end of the pipeline.
                let mut is_terminus = true;
                let mut pipeline_start_name = None;

                if let Some(blob_lock) = &symbol.blob {
                    if let Ok(blob_data) = blob_lock.lock() {
                        if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                            pipeline_start_name = relation_data
                                .pipeline_start
                                .as_ref()
                                .map(|ps| ps.name().to_string());
                        }
                    }
                }

                // Check if any OTHER relation in the symbol table has this relation as continuing_pipeline
                for other_symbol in symbol_table.symbols() {
                    if Arc::ptr_eq(other_symbol, symbol) {
                        continue; // Skip self
                    }

                    if let Some(other_blob) = &other_symbol.blob {
                        if let Ok(other_data) = other_blob.lock() {
                            if let Some(other_rel_data) = other_data.downcast_ref::<RelationData>()
                            {
                                if let Some(continuing) = &other_rel_data.continuing_pipeline {
                                    if Arc::ptr_eq(continuing, symbol) {
                                        // This relation IS someone's continuing_pipeline, so not a terminus
                                        is_terminus = false;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                if is_terminus {
                    subqueries.push(symbol.clone());
                }
            }
        }

        // Add each subquery to its direct parent relation using parent_query_location
        // This matches the C++ approach where subqueries are added when encountered
        for subquery in &subqueries {
            // Get the parent query location from the subquery
            let parent_location = if !subquery.parent_query_location().is_unknown() {
                Some(subquery.parent_query_location())
            } else {
                // Try pipeline_start's parent_query_location
                if let Some(blob_lock) = &subquery.blob {
                    if let Ok(blob_data) = blob_lock.lock() {
                        if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                            if let Some(pipeline_start) = &relation_data.pipeline_start {
                                if !pipeline_start.parent_query_location().is_unknown() {
                                    Some(pipeline_start.parent_query_location())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            if let Some(parent_loc) = parent_location {
                // Look up the parent relation by location
                if let Some(parent_symbol) = symbol_table
                    .lookup_symbol_by_location_and_type(parent_loc.as_ref(), SymbolType::Relation)
                {
                    println!(
                        "DEBUG SUBQUERY: Found parent relation '{}' for subquery '{}' using location",
                        parent_symbol.name(),
                        subquery.name()
                    );

                    // Add to parent's sub_query_pipelines
                    if let Some(blob_lock) = &parent_symbol.blob {
                        if let Ok(mut blob_data) = blob_lock.lock() {
                            if let Some(relation_data) = blob_data.downcast_mut::<RelationData>() {
                                // Check if not already added
                                let already_added = relation_data
                                    .sub_query_pipelines
                                    .iter()
                                    .any(|sq| Arc::ptr_eq(sq, subquery));

                                if !already_added {
                                    println!(
                                        "DEBUG SUBQUERY: Adding '{}' to '{}'",
                                        subquery.name(),
                                        parent_symbol.name()
                                    );
                                    relation_data.sub_query_pipelines.push(subquery.clone());
                                }
                            }
                        }
                    }
                } else {
                    println!(
                        "DEBUG SUBQUERY: WARNING - Could not find parent relation for subquery '{}' at location {:?}",
                        subquery.name(),
                        parent_loc
                    );
                }
            } else {
                println!(
                    "DEBUG SUBQUERY: WARNING - Subquery '{}' has no parent_query_location set",
                    subquery.name()
                );
            }
        }
    }

    /// Gets the symbol table.
    pub fn symbol_table(&self) -> SymbolTable {
        self.symbol_table.clone()
    }

    /// Gets a mutable reference to the symbol table.
    fn symbol_table_mut(&mut self) -> &mut SymbolTable {
        &mut self.symbol_table
    }

    /// Gets the error listener.
    pub fn error_listener(&self) -> Arc<ErrorListener> {
        self.error_listener.clone()
    }

    /// Adds an error message to the error listener.
    pub fn add_error<'a>(
        &self,
        token: &impl std::ops::Deref<Target = GenericToken<std::borrow::Cow<'a, str>>>,
        message: &str,
    ) {
        let location = token_to_location(token);
        self.error_listener.add_error(message.to_string(), location);
    }

    /// Gets the current relation scope, if any.
    pub fn current_relation_scope(&self) -> Option<&Arc<SymbolInfo>> {
        self.current_relation_scope.as_ref()
    }

    /// Sets the current relation scope.
    pub fn set_current_relation_scope(&mut self, scope: Option<Arc<SymbolInfo>>) {
        self.current_relation_scope = scope;
    }

    /// Process a scalar subquery and add it to the symbol table.
    fn process_scalar_subquery(
        &mut self,
        ctx: &ExpressionScalarSubqueryContext<'input>,
    ) -> Option<Arc<SymbolInfo>> {
        // Create a location from the context's start token
        // Get the start token directly - it's not an Option
        let token = ctx.start();
        let location = token_to_location(&token);

        // Define the subquery in the symbol table
        let symbol = self.symbol_table_mut().define_symbol(
            "scalar_subquery".to_string(),
            location,
            SymbolType::Relation,
            None,
            None,
        );

        Some(symbol)
    }

    /// Process a set comparison subquery and add it to the symbol table.
    fn process_set_comparison_subquery(
        &mut self,
        ctx: &ExpressionSetComparisonSubqueryContext<'input>,
    ) -> Option<Arc<SymbolInfo>> {
        // Create a location from the context's start token
        // Get the start token directly - it's not an Option
        let token = ctx.start();
        let location = token_to_location(&token);

        // Define the subquery in the symbol table
        let symbol = self.symbol_table_mut().define_symbol(
            "set_comparison_subquery".to_string(),
            location,
            SymbolType::Relation,
            None,
            None,
        );

        Some(symbol)
    }

    /// Process an IN predicate subquery and add it to the symbol table.
    fn process_in_predicate_subquery(
        &mut self,
        ctx: &ExpressionInPredicateSubqueryContext<'input>,
    ) -> Option<Arc<SymbolInfo>> {
        // Create a location from the context's start token
        // Get the start token directly - it's not an Option
        let token = ctx.start();
        let location = token_to_location(&token);

        // Define the subquery in the symbol table
        let symbol = self.symbol_table_mut().define_symbol(
            "in_predicate_subquery".to_string(),
            location,
            SymbolType::Relation,
            None,
            None,
        );

        Some(symbol)
    }

    /// Process a set predicate subquery and add it to the symbol table.
    fn process_set_predicate_subquery(
        &mut self,
        ctx: &ExpressionSetPredicateSubqueryContext<'input>,
    ) -> Option<Arc<SymbolInfo>> {
        // Create a location from the context's start token
        // Get the start token directly - it's not an Option
        let token = ctx.start();
        let location = token_to_location(&token);

        // Define the subquery in the symbol table
        let symbol = self.symbol_table_mut().define_symbol(
            "set_predicate_subquery".to_string(),
            location,
            SymbolType::Relation,
            None,
            None,
        );

        Some(symbol)
    }

    /// Gets the parent query relation symbol by searching the symbol table.
    /// Returns the parent relation that has this relation in its sub_query_pipelines.
    /// This uses Rust's Arc-based approach rather than C++'s location pointers.
    fn get_parent_query_relation(&self, symbol: &Arc<SymbolInfo>) -> Option<Arc<SymbolInfo>> {
        println!("        get_parent_query_location for '{}'", symbol.name());

        // Check if THIS relation itself has parent_query_index set (meaning it's IN a subquery)
        // Relations that are part of a subquery will have parent_query_index set either:
        // 1. Directly when marked as a subquery root, OR
        // 2. By set_pipeline_start_for_subquery() when walking the continuing_pipeline chain
        if symbol.parent_query_index() < 0 {
            println!("          Not a subquery (parent_query_index < 0)");
            return None;
        }

        println!(
            "          Has parent_query_index: {}",
            symbol.parent_query_index()
        );

        // Use parent_query_location to directly find the parent (like C++ does)
        // This is more reliable than searching through sub_query_pipelines which may not be populated yet
        let parent_location = if !symbol.parent_query_location().is_unknown() {
            Some(symbol.parent_query_location())
        } else {
            // Try pipeline_start's parent_query_location
            if let Some(blob_lock) = &symbol.blob {
                if let Ok(blob_data) = blob_lock.lock() {
                    if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                        if let Some(pipeline_start) = &relation_data.pipeline_start {
                            if !pipeline_start.parent_query_location().is_unknown() {
                                Some(pipeline_start.parent_query_location())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(parent_loc) = parent_location {
            if let Some(parent_symbol) = self
                .symbol_table
                .lookup_symbol_by_location_and_type(parent_loc.as_ref(), SymbolType::Relation)
            {
                println!(
                    "          Found parent query by location: '{}'",
                    parent_symbol.name()
                );
                return Some(parent_symbol);
            }
        }

        println!("          No parent query found using location");
        None
    }

    /// Finds a field reference by name, recursively searching parent relations.
    /// Returns (steps_out, field_index) where steps_out is the number of parent
    /// query levels to traverse, and field_index is the index of the field.
    /// This follows the C++ findFieldReferenceByName pattern.
    fn find_field_reference_by_name(
        &self,
        column_name: &str,
        symbol: &Arc<SymbolInfo>,
    ) -> (usize, Option<usize>) {
        self.find_field_reference_by_name_impl(column_name, symbol, 0)
    }

    fn find_field_reference_by_name_impl(
        &self,
        column_name: &str,
        symbol: &Arc<SymbolInfo>,
        depth: usize,
    ) -> (usize, Option<usize>) {
        // Prevent infinite recursion - limit depth to 10 levels
        if depth > 10 {
            println!(
                "      WARNING: Depth limit reached searching for '{}'",
                column_name
            );
            return (0, None);
        }

        println!(
            "      SubqueryRelationVisitor::find_field_reference_by_name: '{}' in relation '{}' (depth {})",
            column_name,
            symbol.name(),
            depth
        );

        // Search for the field in this relation using lookup_field_index_in_relation
        let field_index = self.lookup_field_index_in_relation(column_name, symbol);

        if field_index.is_some() {
            println!(
                "      Found '{}' in current relation at index {:?}",
                column_name, field_index
            );
            return (0, field_index);
        }

        // Field not found in current relation - check parent query
        println!(
            "      Field '{}' not in current relation, searching parent",
            column_name
        );

        // Get the parent query location (following C++ getParentQueryLocation pattern)
        let parent_location = if !symbol.parent_query_location().is_unknown() {
            Some(symbol.parent_query_location())
        } else {
            // Try pipelineStart
            if let Some(blob_lock) = &symbol.blob {
                if let Ok(blob_data) = blob_lock.lock() {
                    if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                        if let Some(pipeline_start) = &relation_data.pipeline_start {
                            if !pipeline_start.parent_query_location().is_unknown() {
                                Some(pipeline_start.parent_query_location())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };

        // Look up the specific parent symbol by location
        if let Some(parent_loc) = parent_location {
            if let Some(parent_symbol) = self
                .symbol_table
                .lookup_symbol_by_location_and_type(parent_loc.as_ref(), SymbolType::Relation)
            {
                println!("      Found parent relation '{}'", parent_symbol.name());

                // Recursively search in parent (don't hold any locks during recursive call)
                let (parent_steps_out, field_index) =
                    self.find_field_reference_by_name(column_name, &parent_symbol);

                if field_index.is_some() {
                    println!(
                        "      Found '{}' in parent relation '{}', steps_out = {}",
                        column_name,
                        parent_symbol.name(),
                        parent_steps_out + 1
                    );
                    return (parent_steps_out + 1, field_index);
                }
            }
        }

        // Field not found in parent
        (0, None)
    }

    /// Looks up a field index in a specific relation's schema (does NOT walk parent chain).
    fn lookup_field_index_in_relation(
        &self,
        column_name: &str,
        relation_symbol: &Arc<SymbolInfo>,
    ) -> Option<usize> {
        // Parse column name - can be "field" or "schema.field"
        let (schema_name, field_name) = if let Some(dot_pos) = column_name.rfind('.') {
            (&column_name[..dot_pos], &column_name[dot_pos + 1..])
        } else {
            // No schema prefix
            ("", column_name)
        };

        // Get this relation's schema to compare against
        let relation_schema = if let Some(blob_lock) = &relation_symbol.blob {
            if let Ok(blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                    relation_data.schema.clone()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Search ONLY in this relation's field_references that belong to this relation's schema
        if let Some(blob_lock) = &relation_symbol.blob {
            if let Ok(blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                    // Search through field_references in this relation only
                    for (index, field_sym) in relation_data.field_references.iter().enumerate() {
                        // Only match if the field's schema matches THIS relation's schema
                        let field_belongs_to_this_relation = if !schema_name.is_empty() {
                            // Column name has schema prefix - check if field's schema matches
                            if let Some(field_schema) = field_sym.schema() {
                                // Check if field's schema matches the requested schema AND this relation's schema
                                if let Some(ref rel_schema) = relation_schema {
                                    field_schema.name() == schema_name
                                        && Arc::ptr_eq(&field_schema, rel_schema)
                                } else {
                                    // Relation has no schema, match by schema name only
                                    field_schema.name() == schema_name
                                }
                            } else {
                                // Field has no schema, can't match a schema-prefixed name
                                false
                            }
                        } else {
                            // Column name has no schema prefix - match by field name only
                            true
                        };

                        if field_belongs_to_this_relation {
                            if field_sym.name() == column_name || field_sym.name() == field_name {
                                return Some(index);
                            }
                            // Also try matching with schema prefix
                            if let Some(schema) = field_sym.schema() {
                                let qualified_name =
                                    format!("{}.{}", schema.name(), field_sym.name());
                                if qualified_name == column_name {
                                    return Some(index);
                                }
                            }
                        }
                    }

                    // Also check generated_field_references
                    let field_ref_size = relation_data.field_references.len();
                    for (index, field_sym) in
                        relation_data.generated_field_references.iter().enumerate()
                    {
                        if field_sym.name() == column_name || field_sym.name() == field_name {
                            return Some(field_ref_size + index);
                        }
                        if let Some(schema) = field_sym.schema() {
                            let qualified_name = format!("{}.{}", schema.name(), field_sym.name());
                            if qualified_name == column_name {
                                return Some(field_ref_size + index);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Fixes outer references in a relation's expressions.
    /// This traverses the relation proto and updates field references to use outer_reference
    /// when appropriate.
    fn fix_outer_references_in_relation(&self, relation_symbol: &Arc<SymbolInfo>) {
        println!(
            "      Fixing outer references in relation '{}'",
            relation_symbol.name()
        );

        // Only fix outer references if this relation is in a subquery
        if let Some(_parent_symbol) = self.get_parent_query_relation(relation_symbol) {
            println!("        This relation is in a subquery, checking expressions");

            if let Some(blob_lock) = &relation_symbol.blob {
                // First: extract the relation proto (clone it out)
                let relation_proto = if let Ok(blob_data) = blob_lock.lock() {
                    blob_data
                        .downcast_ref::<RelationData>()
                        .map(|relation_data| relation_data.relation.clone())
                } else {
                    None
                };
                // Lock is dropped here

                // Second: fix expressions in the extracted proto
                // Now fix_expression_outer_references can lock relation_symbol as needed
                if let Some(mut relation) = relation_proto {
                    use substrait::proto::rel::RelType;
                    if let Some(rel_type) = &mut relation.rel_type {
                        match rel_type {
                            RelType::Filter(filter_rel) => {
                                if let Some(condition) = &mut filter_rel.condition {
                                    self.fix_expression_outer_references(
                                        condition,
                                        relation_symbol,
                                    );
                                }
                            }
                            RelType::Project(project_rel) => {
                                for expr in &mut project_rel.expressions {
                                    self.fix_expression_outer_references(expr, relation_symbol);
                                }
                            }
                            RelType::Aggregate(agg_rel) => {
                                // Fix grouping expressions
                                for grouping in &mut agg_rel.groupings {
                                    for expr in &mut grouping.grouping_expressions {
                                        self.fix_expression_outer_references(expr, relation_symbol);
                                    }
                                }
                                // Fix measure expressions
                                for measure in &mut agg_rel.measures {
                                    if let Some(measure_func) = &mut measure.measure {
                                        for arg in &mut measure_func.arguments {
                                            if let Some(
                                                substrait::proto::function_argument::ArgType::Value(
                                                    expr,
                                                ),
                                            ) = &mut arg.arg_type
                                            {
                                                self.fix_expression_outer_references(
                                                    expr,
                                                    relation_symbol,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {
                                // Other relation types - we can add support as needed
                            }
                        }
                    }

                    // Third: put the fixed proto back
                    if let Ok(mut blob_data) = blob_lock.lock() {
                        if let Some(relation_data) = blob_data.downcast_mut::<RelationData>() {
                            relation_data.relation = relation;
                        }
                    }
                }
            }
        } else {
            println!("        This relation is NOT in a subquery, skipping");
        }
    }

    /// Recursively fixes outer references in an expression and its sub-expressions.
    fn fix_expression_outer_references(
        &self,
        expr: &mut substrait::proto::Expression,
        relation_symbol: &Arc<SymbolInfo>,
    ) {
        use substrait::proto::expression::RexType;

        match &mut expr.rex_type {
            Some(RexType::Selection(field_ref)) => {
                // This is a field reference - check if it needs to be an outer reference
                if let Some(
                    substrait::proto::expression::field_reference::ReferenceType::DirectReference(
                        ref_seg,
                    ),
                ) = &mut field_ref.reference_type
                {
                    if let Some(
                        substrait::proto::expression::reference_segment::ReferenceType::StructField(
                            struct_field,
                        ),
                    ) = &mut ref_seg.reference_type
                    {
                        // Consume the next stored field info from visit pass
                        let mut current_index = self.expression_field_info_index.borrow_mut();
                        let field_info = self.expression_field_info.borrow();

                        if *current_index < field_info.len() {
                            let (correct_field_index, steps_out) = field_info[*current_index];
                            *current_index += 1;

                            println!(
                                "          Using stored field info: field_index={}, steps_out={}",
                                correct_field_index, steps_out
                            );

                            // Update the field index to the correct value
                            struct_field.field = correct_field_index as i32;

                            // Set outer_reference if steps_out > 0, otherwise root_reference
                            if steps_out > 0 {
                                println!(
                                    "            ✓✓✓ Converting field {} to OUTER REFERENCE (steps_out={})",
                                    correct_field_index, steps_out
                                );
                                field_ref.root_type = Some(
                                    substrait::proto::expression::field_reference::RootType::OuterReference(
                                        substrait::proto::expression::field_reference::OuterReference {
                                            steps_out: steps_out as u32,
                                        },
                                    ),
                                );
                            } else {
                                println!(
                                    "            Field {} is local (steps_out=0)",
                                    correct_field_index
                                );
                                // Ensure it has root_reference
                                if field_ref.root_type.is_none() {
                                    field_ref.root_type = Some(
                                        substrait::proto::expression::field_reference::RootType::RootReference(
                                            substrait::proto::expression::field_reference::RootReference {},
                                        ),
                                    );
                                }
                            }
                        } else {
                            println!(
                                "          WARNING: No stored field info for this expression (index out of bounds)"
                            );
                        }
                    }
                }
            }
            Some(RexType::ScalarFunction(func)) => {
                // Recursively fix arguments
                for arg in &mut func.arguments {
                    if let Some(substrait::proto::function_argument::ArgType::Value(inner_expr)) =
                        &mut arg.arg_type
                    {
                        self.fix_expression_outer_references(inner_expr, relation_symbol);
                    }
                }
            }
            Some(RexType::Cast(cast)) => {
                // Recursively fix the input expression
                if let Some(inner_expr) = &mut cast.input {
                    self.fix_expression_outer_references(inner_expr, relation_symbol);
                }
            }
            _ => {
                // Other expression types - we can add support as needed
            }
        }
    }
}

impl<'input> PlanVisitor<'input> for SubqueryRelationVisitor<'input> {
    fn error_listener(&self) -> Arc<ErrorListener> {
        self.error_listener.clone()
    }

    fn symbol_table(&self) -> SymbolTable {
        self.symbol_table.clone()
    }
}

// ANTLR visitor implementation for SubqueryRelationVisitor
impl<'input> ParseTreeVisitor<'input, SubstraitPlanParserContextType>
    for SubqueryRelationVisitor<'input>
{
}

impl<'input> SubstraitPlanParserVisitor<'input> for SubqueryRelationVisitor<'input> {
    // Override specific visitor methods for subquery processing and expression reprocessing

    fn visit_expressionColumn(&mut self, ctx: &ExpressionColumnContext<'input>) {
        // Re-read column name from parse tree and determine correct field index + steps_out
        // This follows C++ SubqueryRelationVisitor::visitExpressionColumn

        let column_name = ctx.get_text();
        println!(
            "SubqueryRelationVisitor::visit_expressionColumn: '{}'",
            column_name
        );

        // We can only process this if we have a current relation scope
        if let Some(current_rel) = self.current_relation_scope().cloned() {
            // Use find_field_reference_by_name to recursively search for the field
            let (steps_out, field_index) =
                self.find_field_reference_by_name(&column_name, &current_rel);

            if let Some(index) = field_index {
                // Only store field info for relations that are in subqueries
                // This ensures we don't accumulate entries for non-subquery relations
                if self.get_parent_query_relation(&current_rel).is_some() {
                    self.expression_field_info
                        .borrow_mut()
                        .push((index, steps_out));
                    if steps_out > 0 {
                        println!(
                            "      ✓ Stored '{}' as outer reference: field_index={}, steps_out={}",
                            column_name, index, steps_out
                        );
                    } else {
                        println!(
                            "      ✓ Stored '{}' as local reference: field_index={}, steps_out=0",
                            column_name, index
                        );
                    }
                } else {
                    println!(
                        "      ✗ Skipped storing '{}' (not in subquery): field_index={}, steps_out={}",
                        column_name, index, steps_out
                    );
                }
            } else {
                println!(
                    "      WARNING: Field '{}' not found in current or parent relations",
                    column_name
                );
            }
        } else {
            println!("      WARNING: No current relation scope");
        }

        // Continue with default visitor behavior
        self.visit_children(ctx);
    }

    fn visit_relation(&mut self, ctx: &RelationContext<'input>) {
        // Visit all relations and set the current scope for expression processing.
        // This follows the C++ SubqueryRelationVisitor::visitRelation pattern.

        println!(
            "SubqueryRelationVisitor::visit_relation: {}",
            ctx.get_text()
        );

        // Look up the relation symbol (should have been created by previous visitors)
        let token = ctx.start();
        let location = token_to_location(&token);

        if let Some(relation_symbol) = self
            .symbol_table
            .lookup_symbol_by_location_and_type(&location, SymbolType::Relation)
        {
            println!(
                "      Found relation symbol '{}' (location hash: {}), setting as current scope",
                relation_symbol.name(),
                relation_symbol.source_location().location_hash()
            );

            // Save the previous scope
            let old_scope = self.current_relation_scope().cloned();

            // Set this relation as the current scope
            self.set_current_relation_scope(Some(relation_symbol.clone()));

            // Visit children with this scope set
            self.visit_children(ctx);

            // Fix outer references in this relation's expressions
            // This updates field references to use outerReference instead of rootReference
            // when they refer to fields in the parent query
            self.fix_outer_references_in_relation(&relation_symbol);

            // Restore the previous scope
            self.set_current_relation_scope(old_scope);
        } else {
            println!(
                "      WARNING: No relation symbol found at location, visiting children anyway"
            );
            // Visit children even if we couldn't find the symbol
            self.visit_children(ctx);
        }
    }

    fn visit_expressionScalarSubquery(&mut self, ctx: &ExpressionScalarSubqueryContext<'input>) {
        // Process a scalar subquery expression
        println!(
            "SubqueryRelationVisitor processing scalar subquery: {}",
            ctx.get_text()
        );

        // Process the subquery and add it to the symbol table
        if let Some(subquery_symbol) = self.process_scalar_subquery(ctx) {
            // Save the current relation scope
            let old_scope = self.current_relation_scope().cloned();

            // Set the subquery relation as the current scope
            self.set_current_relation_scope(Some(subquery_symbol));

            // Visit children to process subquery details
            self.visit_children(ctx);

            // Restore the old scope
            self.set_current_relation_scope(old_scope);
        } else {
            // Just visit children
            self.visit_children(ctx);
        }
    }

    fn visit_expressionSetComparisonSubquery(
        &mut self,
        ctx: &ExpressionSetComparisonSubqueryContext<'input>,
    ) {
        // Process a set comparison subquery expression
        println!(
            "SubqueryRelationVisitor processing set comparison subquery: {}",
            ctx.get_text()
        );

        // Process the subquery and add it to the symbol table
        if let Some(subquery_symbol) = self.process_set_comparison_subquery(ctx) {
            // Save the current relation scope
            let old_scope = self.current_relation_scope().cloned();

            // Set the subquery relation as the current scope
            self.set_current_relation_scope(Some(subquery_symbol));

            // Visit children to process subquery details
            self.visit_children(ctx);

            // Restore the old scope
            self.set_current_relation_scope(old_scope);
        } else {
            // Just visit children
            self.visit_children(ctx);
        }
    }

    fn visit_expressionInPredicateSubquery(
        &mut self,
        ctx: &ExpressionInPredicateSubqueryContext<'input>,
    ) {
        // Process an IN predicate subquery expression
        println!(
            "SubqueryRelationVisitor processing IN predicate subquery: {}",
            ctx.get_text()
        );

        // Process the subquery and add it to the symbol table
        if let Some(subquery_symbol) = self.process_in_predicate_subquery(ctx) {
            // Save the current relation scope
            let old_scope = self.current_relation_scope().cloned();

            // Set the subquery relation as the current scope
            self.set_current_relation_scope(Some(subquery_symbol));

            // Visit children to process subquery details
            self.visit_children(ctx);

            // Restore the old scope
            self.set_current_relation_scope(old_scope);
        } else {
            // Just visit children
            self.visit_children(ctx);
        }
    }

    fn visit_expressionSetPredicateSubquery(
        &mut self,
        ctx: &ExpressionSetPredicateSubqueryContext<'input>,
    ) {
        // Process a set predicate subquery expression
        println!(
            "SubqueryRelationVisitor processing set predicate subquery: {}",
            ctx.get_text()
        );

        // Process the subquery and add it to the symbol table
        if let Some(subquery_symbol) = self.process_set_predicate_subquery(ctx) {
            // Save the current relation scope
            let old_scope = self.current_relation_scope().cloned();

            // Set the subquery relation as the current scope
            self.set_current_relation_scope(Some(subquery_symbol));

            // Visit children to process subquery details
            self.visit_children(ctx);

            // Restore the old scope
            self.set_current_relation_scope(old_scope);
        } else {
            // Just visit children
            self.visit_children(ctx);
        }
    }

    // We use the default implementation for other visitor methods,
    // which will call visit_children to traverse the tree
}
