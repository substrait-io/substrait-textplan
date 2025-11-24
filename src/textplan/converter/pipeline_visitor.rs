// SPDX-License-Identifier: Apache-2.0

//! Plan visitor implementation for traversing Substrait plans.
//!
//! This module provides implementations of visitors for processing Substrait plans.
//! It builds on the generated BasePlanProtoVisitor trait to provide specialized
//! visitors for different stages of plan processing.

use crate::textplan::common::structured_symbol_data::RelationData;
use crate::textplan::common::ProtoLocation;
use crate::textplan::converter::generated::base_plan_visitor::Traversable;
use crate::textplan::converter::generated::PlanProtoVisitor;
use crate::textplan::SymbolType;
use ::substrait::proto as substrait;
use std::sync::Arc;

/// Pipeline visitor implementation that processes a Substrait plan in multiple stages.
///
/// This visitor builds on the initial visitor and provides more advanced processing
/// of Substrait plans. It can be composed into a chain of visitors, each handling
/// a different aspect of plan processing.
pub struct PipelineVisitor {
    /// Symbol table for storing plan element information
    symbol_table: crate::textplan::symbol_table::SymbolTable,
    /// Current relation context for scope resolution
    current_relation_scope: Option<Arc<crate::textplan::SymbolInfo>>,
    /// Previous relation scope (saved for restoration)
    previous_relation_scope: Option<Arc<crate::textplan::SymbolInfo>>,
    /// Current location in the protocol buffer structure
    current_location: ProtoLocation,
    /// Flag to prevent infinite recursion when traversing subquery relations
    in_subquery_traversal: bool,
}

impl PipelineVisitor {
    /// Create a new pipeline visitor with the given symbol table
    pub fn new(symbol_table: crate::textplan::symbol_table::SymbolTable) -> Self {
        Self {
            symbol_table,
            current_relation_scope: None,
            previous_relation_scope: None,
            current_location: ProtoLocation::default(),
            in_subquery_traversal: false,
        }
    }

    /// Get the symbol table built by this visitor
    pub fn symbol_table(&self) -> &crate::textplan::symbol_table::SymbolTable {
        &self.symbol_table
    }

    /// Get a mutable reference to the symbol table
    pub fn symbol_table_mut(&mut self) -> &mut crate::textplan::symbol_table::SymbolTable {
        &mut self.symbol_table
    }

    pub fn visit_extended_expression(&mut self, obj: &substrait::ExtendedExpression) {
        obj.traverse(self);
    }

    pub fn visit_plan(&mut self, obj: &substrait::Plan) {
        obj.traverse(self);
    }

    /// Helper to traverse expressions looking for subqueries
    /// This avoids stack overflow by not using the full traverse mechanism
    fn traverse_expression_for_subqueries(&mut self, expr: &substrait::Expression) {
        use substrait::expression::RexType;

        // Process this expression
        self.post_process_expression(expr);

        // Recursively check nested expressions
        if let Some(rex_type) = &expr.rex_type {
            match rex_type {
                RexType::Subquery(subquery) => {
                    // Prevent infinite recursion - don't traverse nested subqueries
                    if self.in_subquery_traversal {
                        return;
                    }

                    // For subquery expressions, we need to traverse the relation tree inside
                    use substrait::expression::subquery::SubqueryType;

                    let (rel_opt, field_path) = match &subquery.subquery_type {
                        Some(SubqueryType::Scalar(scalar)) => {
                            (scalar.input.as_ref(), "subquery.scalar.input")
                        }
                        Some(SubqueryType::InPredicate(in_pred)) => {
                            (in_pred.haystack.as_ref(), "subquery.in_predicate.haystack")
                        }
                        Some(SubqueryType::SetPredicate(set_pred)) => {
                            (set_pred.tuples.as_ref(), "subquery.set_predicate.tuples")
                        }
                        Some(SubqueryType::SetComparison(set_comp)) => {
                            (set_comp.right.as_ref(), "subquery.set_comparison.right")
                        }
                        None => (None, ""),
                    };

                    if let Some(rel) = rel_opt {
                        // Traverse the subquery relation tree
                        let prev_loc = self.current_location().clone();
                        let mut subquery_loc = self.current_location().clone();
                        for field_name in field_path.split('.') {
                            subquery_loc = subquery_loc.field(field_name);
                        }
                        self.set_location(subquery_loc);

                        // Set flag to prevent recursive subquery traversal
                        self.in_subquery_traversal = true;
                        self.traverse_subquery_relation(rel);
                        self.in_subquery_traversal = false;

                        self.set_location(prev_loc);
                    }
                }
                RexType::ScalarFunction(func) => {
                    for (i, arg) in func.arguments.iter().enumerate() {
                        let prev_loc = self.current_location().clone();
                        self.set_location(
                            self.current_location()
                                .field("scalar_function")
                                .indexed_field("arguments", i),
                        );
                        self.post_process_function_argument(arg);
                        self.set_location(prev_loc);
                    }
                }
                RexType::Cast(cast) => {
                    if let Some(input) = &cast.input {
                        let prev_loc = self.current_location().clone();
                        self.set_location(self.current_location().field("cast").field("input"));
                        self.traverse_expression_for_subqueries(input);
                        self.set_location(prev_loc);
                    }
                }
                // Add other expression types that can contain nested expressions as needed
                _ => {}
            }
        }
    }

    /// Helper to traverse a subquery relation tree and set up pipeline connections
    fn traverse_subquery_relation(&mut self, rel: &substrait::Rel) {
        // Set up continuing_pipeline for this relation by looking at its input
        let symbol = self
            .symbol_table()
            .lookup_symbol_by_location_and_type(self.current_location(), SymbolType::Relation);

        if let Some(symbol_ref) = &symbol {
            symbol_ref.with_blob::<RelationData, _, _>(|relation_data| {
                use substrait::rel::RelType;
                match &rel.rel_type {
                    Some(RelType::Aggregate(_)) => {
                        let rel_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                            &self.current_location().field("aggregate").field("input"),
                            SymbolType::Relation,
                        );
                        relation_data.continuing_pipeline = rel_symbol;
                    }
                    Some(RelType::Filter(_)) => {
                        let rel_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                            &self.current_location().field("filter").field("input"),
                            SymbolType::Relation,
                        );
                        relation_data.continuing_pipeline = rel_symbol;
                    }
                    Some(RelType::Project(_)) => {
                        let rel_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                            &self.current_location().field("project").field("input"),
                            SymbolType::Relation,
                        );
                        relation_data.continuing_pipeline = rel_symbol;
                    }
                    Some(RelType::Cross(_)) => {
                        let left_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                            &self.current_location().field("cross").field("left"),
                            SymbolType::Relation,
                        );
                        let right_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                            &self.current_location().field("cross").field("right"),
                            SymbolType::Relation,
                        );
                        if let Some(left) = left_symbol {
                            relation_data.new_pipelines.push(left);
                        }
                        if let Some(right) = right_symbol {
                            relation_data.new_pipelines.push(right);
                        }
                    }
                    _ => {}
                }
            });
        }

        // Then recursively traverse inputs
        use substrait::rel::RelType;
        if let Some(rel_type) = &rel.rel_type {
            match rel_type {
                RelType::Aggregate(agg) => {
                    if let Some(input) = &agg.input {
                        let prev_loc = self.current_location().clone();
                        self.set_location(
                            self.current_location().field("aggregate").field("input"),
                        );
                        self.traverse_subquery_relation(input);
                        self.set_location(prev_loc);
                    }
                }
                RelType::Filter(filter) => {
                    // Traverse the filter condition to find nested subqueries
                    if let Some(condition) = &filter.condition {
                        let prev_loc = self.current_location().clone();
                        self.set_location(
                            self.current_location().field("filter").field("condition"),
                        );
                        self.traverse_expression_for_subqueries(condition);
                        self.set_location(prev_loc);
                    }
                    // Then traverse the input relation
                    if let Some(input) = &filter.input {
                        let prev_loc = self.current_location().clone();
                        self.set_location(self.current_location().field("filter").field("input"));
                        self.traverse_subquery_relation(input);
                        self.set_location(prev_loc);
                    }
                }
                RelType::Project(project) => {
                    if let Some(input) = &project.input {
                        let prev_loc = self.current_location().clone();
                        self.set_location(self.current_location().field("project").field("input"));
                        self.traverse_subquery_relation(input);
                        self.set_location(prev_loc);
                    }
                }
                RelType::Cross(cross) => {
                    if let Some(left) = &cross.left {
                        let prev_loc = self.current_location().clone();
                        self.set_location(self.current_location().field("cross").field("left"));
                        self.traverse_subquery_relation(left);
                        self.set_location(prev_loc);
                    }
                    if let Some(right) = &cross.right {
                        let prev_loc = self.current_location().clone();
                        self.set_location(self.current_location().field("cross").field("right"));
                        self.traverse_subquery_relation(right);
                        self.set_location(prev_loc);
                    }
                }
                _ => {}
            }
        }
    }
}

impl PlanProtoVisitor for PipelineVisitor {
    fn current_location(&self) -> &ProtoLocation {
        &self.current_location
    }

    fn set_location(&mut self, location: ProtoLocation) {
        self.current_location = location;
    }

    fn pre_process_rel(&mut self, _rel: &substrait::Rel) {
        // Set current_relation_scope before visiting children so expressions can access it
        // Save the previous scope for restoration in post_process_rel
        self.previous_relation_scope = self.current_relation_scope.clone();

        let symbol = self
            .symbol_table()
            .lookup_symbol_by_location_and_type(self.current_location(), SymbolType::Relation);

        self.current_relation_scope = symbol.clone();
    }

    fn post_process_rel(&mut self, rel: &substrait::Rel) {
        let symbol = self
            .symbol_table()
            .lookup_symbol_by_location_and_type(self.current_location(), SymbolType::Relation);

        // TODO -- Consider using rel_type_case to simplify this block.
        // Process the relation data before changing the scope back
        if let Some(symbol_ref) = &symbol {
            symbol_ref.with_blob::<RelationData, _, _>(|relation_data| match &rel.rel_type {
                Some(substrait::rel::RelType::Read(_)) => {
                    // No relations beyond this one.
                }
                Some(substrait::rel::RelType::Filter(_)) => {
                    let rel_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("filter").field("input"),
                        SymbolType::Relation,
                    );
                    relation_data.continuing_pipeline = rel_symbol;
                }
                Some(substrait::rel::RelType::Fetch(_)) => {
                    let rel_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("fetch").field("input"),
                        SymbolType::Relation,
                    );
                    relation_data.continuing_pipeline = rel_symbol;
                }
                Some(substrait::rel::RelType::Aggregate(_)) => {
                    let rel_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("aggregate").field("input"),
                        SymbolType::Relation,
                    );
                    relation_data.continuing_pipeline = rel_symbol;
                }
                Some(substrait::rel::RelType::Sort(_)) => {
                    let rel_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("sort").field("input"),
                        SymbolType::Relation,
                    );
                    relation_data.continuing_pipeline = rel_symbol;
                }
                Some(substrait::rel::RelType::Join(_)) => {
                    let left_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("join").field("left"),
                        SymbolType::Relation,
                    );
                    let right_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("join").field("right"),
                        SymbolType::Relation,
                    );
                    relation_data.new_pipelines.push(left_symbol.unwrap());
                    relation_data.new_pipelines.push(right_symbol.unwrap());
                }
                Some(substrait::rel::RelType::Project(_)) => {
                    let rel_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("project").field("input"),
                        SymbolType::Relation,
                    );
                    relation_data.continuing_pipeline = rel_symbol;
                }
                Some(substrait::rel::RelType::Set(set)) => {
                    for i in 0..set.inputs.len() {
                        let input_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                            &self
                                .current_location()
                                .field("set")
                                .indexed_field("inputs", i),
                            SymbolType::Relation,
                        );
                        relation_data.new_pipelines.push(input_symbol.unwrap());
                    }
                }
                Some(substrait::rel::RelType::ExtensionSingle(_)) => {
                    let rel_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self
                            .current_location()
                            .field("extension_single")
                            .field("input"),
                        SymbolType::Relation,
                    );
                    relation_data.continuing_pipeline = rel_symbol;
                }
                Some(substrait::rel::RelType::ExtensionMulti(multi)) => {
                    for i in 0..multi.inputs.len() {
                        let input_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                            &self
                                .current_location()
                                .field("extension_multi")
                                .indexed_field("inputs", i),
                            SymbolType::Relation,
                        );
                        relation_data.new_pipelines.push(input_symbol.unwrap());
                    }
                }
                Some(substrait::rel::RelType::ExtensionLeaf(_)) => {
                    // No children.
                }
                Some(substrait::rel::RelType::Cross(_)) => {
                    let left_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("cross").field("left"),
                        SymbolType::Relation,
                    );
                    let right_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("cross").field("right"),
                        SymbolType::Relation,
                    );
                    relation_data.new_pipelines.push(left_symbol.unwrap());
                    relation_data.new_pipelines.push(right_symbol.unwrap());
                }
                Some(substrait::rel::RelType::Reference(_)) => {
                    // TODO -- Add support for references in text plans.
                    todo!()
                }
                Some(substrait::rel::RelType::Write(_)) => {
                    let rel_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("write").field("input"),
                        SymbolType::Relation,
                    );
                    relation_data.continuing_pipeline = rel_symbol;
                }
                Some(substrait::rel::RelType::Ddl(_)) => {
                    // TODO -- Add support for DDL in text plans.
                    todo!()
                }
                Some(substrait::rel::RelType::Update(_)) => {
                    // TODO -- Add support for update in text plans.
                    todo!()
                }
                Some(substrait::rel::RelType::HashJoin(_)) => {
                    let left_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("hash_join").field("left"),
                        SymbolType::Relation,
                    );
                    let right_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("hash_join").field("right"),
                        SymbolType::Relation,
                    );
                    relation_data.new_pipelines.push(left_symbol.unwrap());
                    relation_data.new_pipelines.push(right_symbol.unwrap());
                }
                Some(substrait::rel::RelType::MergeJoin(_)) => {
                    let left_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("merge_join").field("left"),
                        SymbolType::Relation,
                    );
                    let right_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("merge_join").field("right"),
                        SymbolType::Relation,
                    );
                    relation_data.new_pipelines.push(left_symbol.unwrap());
                    relation_data.new_pipelines.push(right_symbol.unwrap());
                }
                Some(substrait::rel::RelType::NestedLoopJoin(_)) => {
                    let left_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self
                            .current_location()
                            .field("nested_loop_join")
                            .field("left"),
                        SymbolType::Relation,
                    );
                    let right_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self
                            .current_location()
                            .field("nested_loop_join")
                            .field("right"),
                        SymbolType::Relation,
                    );
                    relation_data.new_pipelines.push(left_symbol.unwrap());
                    relation_data.new_pipelines.push(right_symbol.unwrap());
                }
                Some(substrait::rel::RelType::Window(_)) => {
                    let rel_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("window").field("input"),
                        SymbolType::Relation,
                    );
                    relation_data.continuing_pipeline = rel_symbol;
                }
                Some(substrait::rel::RelType::Exchange(_)) => {
                    let rel_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("exchange").field("input"),
                        SymbolType::Relation,
                    );
                    relation_data.continuing_pipeline = rel_symbol;
                }
                Some(substrait::rel::RelType::Expand(_)) => {
                    let rel_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                        &self.current_location().field("expand").field("input"),
                        SymbolType::Relation,
                    );
                    relation_data.continuing_pipeline = rel_symbol;
                }
                None => {}
            });
        }

        // Restore the previous scope
        self.current_relation_scope = self.previous_relation_scope.clone();
    }

    fn post_process_plan_rel(&mut self, relation: &substrait::PlanRel) {
        let symbols = self
            .symbol_table()
            .lookup_symbols_by_location(self.current_location());
        // A symbol is guaranteed as we previously visited the parse tree.
        symbols[0].with_blob::<RelationData, _, _>(|relation_data| match relation.rel_type {
            Some(substrait::plan_rel::RelType::Rel(_)) => {
                let rel_symbol = self.symbol_table.lookup_symbol_by_location_and_type(
                    &self.current_location().field("rel"),
                    SymbolType::Relation,
                );
                relation_data.new_pipelines.push(rel_symbol.unwrap());
            }
            Some(substrait::plan_rel::RelType::Root(_)) => {
                let lookup_location = self.current_location().field("root").field("input");
                let input_symbol = self
                    .symbol_table
                    .lookup_symbol_by_location_and_type(&lookup_location, SymbolType::Relation);
                relation_data.new_pipelines.push(input_symbol.unwrap());
            }
            None => {}
        });
    }

    fn post_process_function_argument(&mut self, arg: &substrait::FunctionArgument) {
        // Workaround: The generated visitor doesn't traverse FunctionArgument.arg_type
        // We need to manually traverse expressions inside function arguments to find subqueries
        if let Some(arg_type) = &arg.arg_type {
            if let substrait::function_argument::ArgType::Value(expr) = arg_type {
                // Save current location and update for the value field
                let prev_location = self.current_location().clone();
                self.set_location(self.current_location().field("value"));

                // Manually traverse this specific expression and its children
                // This is safe because the generated visitor doesn't do this traversal
                self.traverse_expression_for_subqueries(expr);

                self.set_location(prev_location);
            }
        }
    }

    fn post_process_expression(&mut self, expression: &substrait::Expression) {
        println!(
            "DEBUG PIPELINE: post_process_expression called at {}",
            self.current_location().path_string()
        );
        if let Some(rex_type) = &expression.rex_type {
            if let substrait::expression::RexType::Subquery(subquery) = rex_type {
                use substrait::expression::subquery::SubqueryType;

                // Determine the subquery relation location based on subquery type
                let subquery_rel_path = match &subquery.subquery_type {
                    Some(SubqueryType::Scalar(_)) => "subquery.scalar.input",
                    Some(SubqueryType::InPredicate(_)) => "subquery.in_predicate.haystack",
                    Some(SubqueryType::SetPredicate(_)) => "subquery.set_predicate.tuples",
                    Some(SubqueryType::SetComparison(_)) => "subquery.set_comparison.right",
                    None => return, // No subquery type set
                };

                // Build the location for the subquery relation
                let mut subquery_location = self.current_location().clone();
                for field_name in subquery_rel_path.split('.') {
                    subquery_location = subquery_location.field(field_name);
                }

                // Look up the subquery relation symbol (the terminus)
                let subquery_symbol = self
                    .symbol_table
                    .lookup_symbol_by_location_and_type(&subquery_location, SymbolType::Relation);

                if let Some(subquery_sym) = subquery_symbol {
                    // Add the subquery to the current relation's sub_query_pipelines
                    if let Some(current_rel) = &self.current_relation_scope {
                        current_rel.with_blob::<RelationData, _, _>(|relation_data| {
                            println!(
                                "DEBUG PIPELINE: Adding subquery '{}' to relation '{}'",
                                subquery_sym.name(),
                                current_rel.name()
                            );
                            relation_data.sub_query_pipelines.push(subquery_sym.clone());
                        });

                        // Set parent query index to mark this as a subquery
                        // Only set if not already set (InitialPlanVisitor sets the proper index)
                        if subquery_sym.parent_query_index() < 0 {
                            println!(
                                "DEBUG PIPELINE: Setting parent_query_index for '{}' (parent: '{}') to 0",
                                subquery_sym.name(),
                                current_rel.name()
                            );
                            self.symbol_table.set_parent_query_index(&subquery_sym, 0);
                        } else {
                            println!(
                                "DEBUG PIPELINE: parent_query_index for '{}' (parent: '{}') already set to {}",
                                subquery_sym.name(),
                                current_rel.name(),
                                subquery_sym.parent_query_index()
                            );
                        }
                    }

                    // Set pipeline_start on the terminus (subquery relation itself)
                    // The rest of the pipeline will be populated by populate_subquery_pipelines()
                    // after all relations have been visited and continuing_pipeline is set up
                    subquery_sym.with_blob::<RelationData, _, _>(|relation_data| {
                        println!(
                            "DEBUG PIPELINE: Setting pipeline_start on terminus '{}' to itself",
                            subquery_sym.name()
                        );
                        relation_data.pipeline_start = Some(subquery_sym.clone());
                    });

                    // Now traverse the subquery relation tree to set up continuing_pipeline connections
                    // Get the actual relation from the subquery
                    let subquery_rel = match &subquery.subquery_type {
                        Some(SubqueryType::Scalar(scalar)) => scalar.input.as_ref(),
                        Some(SubqueryType::InPredicate(pred)) => pred.haystack.as_ref(),
                        Some(SubqueryType::SetPredicate(pred)) => pred.tuples.as_ref(),
                        Some(SubqueryType::SetComparison(comp)) => comp.right.as_ref(),
                        None => None,
                    };

                    if let Some(rel) = subquery_rel {
                        let prev_loc = self.current_location().clone();
                        self.set_location(subquery_location);
                        self.traverse_subquery_relation(rel);
                        self.set_location(prev_loc);
                    }
                }
            }
        }
    }
}
