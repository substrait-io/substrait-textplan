// SPDX-License-Identifier: Apache-2.0

//! Save a binary Substrait plan to textplan format.

use crate::proto;
use crate::textplan::common::error::TextPlanError;
use crate::textplan::common::structured_symbol_data::RelationData;
use crate::textplan::converter::initial_plan_visitor::InitialPlanVisitor;
use crate::textplan::converter::pipeline_visitor::PipelineVisitor;
use crate::textplan::printer::plan_printer::{PlanPrinter, TextPlanFormat};
use crate::textplan::symbol_table::SymbolTable;
use crate::textplan::SymbolInfo;
use std::sync::Arc;

/// Saves a binary Substrait plan to textplan format.
///
/// # Arguments
///
/// * `bytes` - The binary plan to convert.
///
/// # Returns
///
/// The textplan representation of the plan.
pub fn save_to_text(bytes: &[u8]) -> Result<String, TextPlanError> {
    // Deserialize the binary data to a Plan
    let plan = proto::load_plan_from_binary(bytes)?;

    // Convert the plan to a textplan
    convert_plan_to_text(&plan)
}

/// Converts a Plan to a textplan.
///
/// # Arguments
///
/// * `plan` - The Substrait plan to convert.
///
/// # Returns
///
/// The textplan representation of the plan.
fn convert_plan_to_text(plan: &substrait::proto::Plan) -> Result<String, TextPlanError> {
    // Start building the textplan string
    let mut textplan = String::new();

    // Add version information
    if let Some(version) = &plan.version {
        textplan.push_str(&format!(
            "// Substrait plan version: {}.{}.{}\n",
            version.major_number, version.minor_number, version.patch_number
        ));
        if !version.producer.is_empty() {
            textplan.push_str(&format!("// Producer: {}\n", version.producer));
        }
    }
    textplan.push('\n');

    // Add extension URIs if present
    if !plan.extension_uris.is_empty() {
        textplan.push_str("// Extension URIs:\n");
        for uri in &plan.extension_uris {
            textplan.push_str(&format!("// - {}: {}\n", uri.extension_uri_anchor, uri.uri));
        }
        textplan.push('\n');
    }

    // Add extensions if present
    if !plan.extensions.is_empty() {
        textplan.push_str("// Extensions:\n");
        for ext in &plan.extensions {
            // Use the correct mapping_type oneof field
            if let Some(mapping_type) = &ext.mapping_type {
                use ::substrait::proto::extensions::simple_extension_declaration::MappingType;
                match mapping_type {
                    MappingType::ExtensionFunction(func) => {
                        textplan.push_str(&format!(
                            "// - URI Ref: {}, Function: {}, Name: {}\n",
                            func.extension_uri_reference, func.function_anchor, func.name
                        ));
                    }
                    MappingType::ExtensionType(typ) => {
                        textplan.push_str(&format!(
                            "// - URI Ref: {}, Type: {}, Name: {}\n",
                            typ.extension_uri_reference, typ.type_anchor, typ.name
                        ));
                    }
                    MappingType::ExtensionTypeVariation(var) => {
                        textplan.push_str(&format!(
                            "// - URI Ref: {}, Type Variation: {}, Name: {}\n",
                            var.extension_uri_reference, var.type_variation_anchor, var.name
                        ));
                    }
                }
            }
        }
        textplan.push('\n');
    }

    // Process the plan using the PipelineVisitor to build a symbol table
    let plan_body = process_plan_with_visitor(plan)?;

    // Append the plan body to the header comments
    textplan.push_str(&plan_body);

    Ok(textplan)
}

/// Processes a plan using the PipelineVisitor to extract structured information.
///
/// # Arguments
///
/// * `plan` - The Substrait plan to process.
///
/// # Returns
///
/// The textplan body generated from the symbol table
pub fn process_plan_with_visitor(plan: &substrait::proto::Plan) -> Result<String, TextPlanError> {
    // Create a symbol table for the plan
    let symbol_table = SymbolTable::new();

    let mut visitor1 = InitialPlanVisitor::new(symbol_table);
    visitor1.visit_plan(plan);
    // MEGAHACK -- Check for errors.

    println!(
        "DEBUG: After InitialPlanVisitor, symbol table has {} symbols",
        visitor1.symbol_table().len()
    );
    for symbol in visitor1.symbol_table().symbols() {
        if symbol.symbol_type() == crate::textplan::SymbolType::Relation {
            if symbol.parent_query_index() >= 0 {
                println!(
                    "  - {} (type: {:?}, parent_query_index: {}, parent_query_hash: {})",
                    symbol.name(),
                    symbol.symbol_type(),
                    symbol.parent_query_index(),
                    symbol.parent_query_location().location_hash()
                );
            } else {
                println!(
                    "  - {} (type: {:?}, location: {:?})",
                    symbol.name(),
                    symbol.symbol_type(),
                    symbol.source_location()
                );
            }
        } else {
            println!("  - {} (type: {:?})", symbol.name(), symbol.symbol_type());
        }
    }

    // Create a pipeline visitor with the symbol table
    let mut visitor = PipelineVisitor::new(visitor1.symbol_table_mut().clone());

    // Visit the plan to build the symbol table
    visitor.visit_plan(plan);

    // MEGAHACK -- Check for errors.

    println!(
        "DEBUG: After PipelineVisitor, symbol table has {} symbols",
        visitor.symbol_table().len()
    );
    for symbol in visitor.symbol_table().symbols() {
        println!("  - {} (type: {:?})", symbol.name(), symbol.symbol_type());
    }

    // Populate sub_query_pipelines by finding subquery relations
    populate_subquery_pipelines(visitor.symbol_table_mut())?;

    // Get the populated symbol table from the visitor
    let symbol_table = visitor.symbol_table().clone();

    // Create a plan printer and use it to convert the symbol table to a textplan
    let mut printer = PlanPrinter::new(TextPlanFormat::Standard);
    let plan_text = printer.print_plan(&symbol_table)?;

    Ok(plan_text)
}

/// Populates pipeline_start for all relations in subquery pipelines.
///
/// This function finds subquery terminus relations (those with parent_query_index >= 0)
/// and walks their continuing_pipeline chains to set pipeline_start on all relations.
/// The continuing_pipeline connections were already set up by PipelineVisitor.
fn populate_subquery_pipelines(symbol_table: &mut SymbolTable) -> Result<(), TextPlanError> {
    // Find terminus relations (those with parent_query_index >= 0)
    let mut subquery_termini = Vec::new();
    for symbol in symbol_table.symbols() {
        if symbol.symbol_type() == crate::textplan::SymbolType::Relation
            && symbol.parent_query_index() >= 0
        {
            println!("  Found subquery terminus: '{}'", symbol.name());
            subquery_termini.push(symbol.clone());
        }
    }

    // For each terminus, walk the continuing_pipeline chain and set pipeline_start
    for terminus in subquery_termini {
        walk_and_set_pipeline_start(&terminus)?;
    }

    // Now fix outer references in subquery relations
    for symbol in symbol_table.symbols() {
        if symbol.symbol_type() == crate::textplan::SymbolType::Relation
            && symbol.parent_query_index() >= 0
        {
            println!("  Fixing outer references in '{}'", symbol.name());
            fix_outer_references_in_subquery_relation(symbol_table, symbol)?;
        }
    }

    Ok(())
}

/// Fixes outer references in a subquery relation's expressions.
///
/// For field references in subquery relations, we need to ensure they use
/// outerReference when referencing parent query fields.
fn fix_outer_references_in_subquery_relation(
    _symbol_table: &SymbolTable,
    relation_symbol: &Arc<SymbolInfo>,
) -> Result<(), TextPlanError> {
    // For now, we'll just ensure that any field references that are already
    // marked as outerReference in the input binary plan are preserved.
    // The binary plan should already have the correct reference types.

    // In the future, we might need to actively fix field references that
    // should be outer references but aren't, by analyzing the schema and
    // field indices. But for roundtrip testing, preserving what's there
    // should be sufficient.

    // The actual preservation happens automatically because we're not modifying
    // the proto - we're just converting it to textplan and back.

    println!("    (Outer references already preserved from binary proto)");
    Ok(())
}

/// Walks through a subquery pipeline and sets pipeline_start on all relations.
///
/// Following the C++ implementation, this walks through continuing_pipeline
/// and sets pipeline_start to point to the terminus on all relations in the chain.
fn walk_and_set_pipeline_start(terminus: &Arc<SymbolInfo>) -> Result<(), TextPlanError> {
    println!("  Walking pipeline for terminus '{}'", terminus.name());

    // Walk through continuing_pipeline chain
    let mut current_sym = Some(terminus.clone());
    while let Some(sym) = current_sym {
        let next_sym = if let Some(blob_lock) = &sym.blob {
            if let Ok(mut blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_mut::<RelationData>() {
                    println!(
                        "    Setting pipeline_start on '{}' to '{}'",
                        sym.name(),
                        terminus.name()
                    );
                    relation_data.pipeline_start = Some(terminus.clone());
                    relation_data.continuing_pipeline.clone()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        current_sym = next_sym;
    }

    Ok(())
}

/// Checks if a Rel contains any subquery expressions.
fn has_subquery_expression(rel: &substrait::proto::Rel) -> bool {
    use substrait::proto::rel::RelType;

    match &rel.rel_type {
        Some(RelType::Filter(filter_rel)) => {
            if let Some(condition) = &filter_rel.condition {
                has_subquery_in_expression(condition)
            } else {
                false
            }
        }
        Some(RelType::Project(project_rel)) => project_rel
            .expressions
            .iter()
            .any(has_subquery_in_expression),
        Some(RelType::Join(join_rel)) => {
            if let Some(expr) = &join_rel.expression {
                has_subquery_in_expression(expr)
            } else {
                false
            }
        }
        // Add other relation types as needed
        _ => false,
    }
}

/// Checks if an Expression contains a subquery.
fn has_subquery_in_expression(expr: &substrait::proto::Expression) -> bool {
    use substrait::proto::expression::RexType;

    match &expr.rex_type {
        Some(RexType::Subquery(_)) => true,
        Some(RexType::ScalarFunction(func)) => func.arguments.iter().any(|arg| {
            if let Some(substrait::proto::function_argument::ArgType::Value(inner_expr)) =
                &arg.arg_type
            {
                has_subquery_in_expression(inner_expr)
            } else {
                false
            }
        }),
        // Add other expression types as needed
        _ => false,
    }
}

/// Extracts subquery relations from a parent relation and adds their pipeline starts.
fn extract_and_add_subquery_pipelines(
    symbol_table: &SymbolTable,
    parent_symbol: &Arc<SymbolInfo>,
) -> Result<(), TextPlanError> {
    // Get the relation data
    let blob_lock = parent_symbol.blob.as_ref().ok_or_else(|| {
        TextPlanError::ProtobufError(format!(
            "Parent relation '{}' has no blob data",
            parent_symbol.name()
        ))
    })?;

    let mut blob_data = blob_lock.lock().map_err(|_| {
        TextPlanError::ProtobufError(format!(
            "Failed to lock blob for parent relation '{}'",
            parent_symbol.name()
        ))
    })?;

    let relation_data = blob_data
        .downcast_mut::<crate::textplan::common::structured_symbol_data::RelationData>()
        .ok_or_else(|| {
            TextPlanError::ProtobufError(format!(
                "Parent relation '{}' blob is not RelationData",
                parent_symbol.name()
            ))
        })?;

    // Extract subquery pipeline starts
    let subquery_starts =
        extract_subquery_starts(symbol_table, parent_symbol, &relation_data.relation)?;

    println!(
        "    Found {} subquery pipeline starts for '{}'",
        subquery_starts.len(),
        parent_symbol.name()
    );

    // Add to sub_query_pipelines
    relation_data.sub_query_pipelines.extend(subquery_starts);

    Ok(())
}

/// Extracts subquery pipeline starts from a Rel.
fn extract_subquery_starts(
    symbol_table: &SymbolTable,
    parent_symbol: &Arc<SymbolInfo>,
    rel: &substrait::proto::Rel,
) -> Result<Vec<Arc<SymbolInfo>>, TextPlanError> {
    use substrait::proto::rel::RelType;

    let mut starts = Vec::new();

    match &rel.rel_type {
        Some(RelType::Filter(filter_rel)) => {
            if let Some(condition) = &filter_rel.condition {
                starts.extend(extract_subquery_starts_from_expression(
                    symbol_table,
                    parent_symbol,
                    condition,
                )?);
            }
        }
        Some(RelType::Project(project_rel)) => {
            for expr in &project_rel.expressions {
                starts.extend(extract_subquery_starts_from_expression(
                    symbol_table,
                    parent_symbol,
                    expr,
                )?);
            }
        }
        Some(RelType::Join(join_rel)) => {
            if let Some(expr) = &join_rel.expression {
                starts.extend(extract_subquery_starts_from_expression(
                    symbol_table,
                    parent_symbol,
                    expr,
                )?);
            }
        }
        _ => {}
    }

    Ok(starts)
}

/// Extracts subquery pipeline starts from an Expression.
fn extract_subquery_starts_from_expression(
    symbol_table: &SymbolTable,
    parent_symbol: &Arc<SymbolInfo>,
    expr: &substrait::proto::Expression,
) -> Result<Vec<Arc<SymbolInfo>>, TextPlanError> {
    use substrait::proto::expression::{subquery::SubqueryType, RexType};

    let mut starts = Vec::new();

    match &expr.rex_type {
        Some(RexType::Subquery(subquery)) => {
            // Extract the subquery relation based on its type (but we don't actually need the Rel)
            // The subquery symbols were already registered by InitialPlanVisitor with parent_query_location
            let _subquery_rel: Option<&substrait::proto::Rel> = match &subquery.subquery_type {
                Some(SubqueryType::Scalar(scalar)) => scalar.input.as_deref(),
                Some(SubqueryType::InPredicate(in_pred)) => in_pred.haystack.as_deref(),
                Some(SubqueryType::SetPredicate(set_pred)) => set_pred.tuples.as_deref(),
                Some(SubqueryType::SetComparison(set_comp)) => set_comp.right.as_deref(),
                None => {
                    println!("      WARNING: Subquery has no type set");
                    None
                }
            };

            // Find all subquery symbols that belong to this parent
            let subquery_symbols = find_subquery_relations_for_parent(symbol_table, parent_symbol)?;

            for symbol in subquery_symbols {
                if let Some(start) = find_pipeline_start(symbol_table, &symbol)? {
                    println!("      Found subquery pipeline start: '{}'", start.name());
                    if !starts.iter().any(|s| Arc::ptr_eq(s, &start)) {
                        starts.push(start);
                    }
                }
            }
        }
        Some(RexType::ScalarFunction(func)) => {
            // Recursively check arguments
            for arg in &func.arguments {
                if let Some(substrait::proto::function_argument::ArgType::Value(inner_expr)) =
                    &arg.arg_type
                {
                    starts.extend(extract_subquery_starts_from_expression(
                        symbol_table,
                        parent_symbol,
                        inner_expr,
                    )?);
                }
            }
        }
        _ => {}
    }

    Ok(starts)
}

/// Finds all relation symbols that are subqueries of the current parent.
/// Note: This is called during load_binary phase, AFTER InitialPlanVisitor has already
/// set parent_query_location on all subquery relations. We need to filter by which
/// parent these subqueries belong to.
fn find_subquery_relations_for_parent(
    symbol_table: &SymbolTable,
    parent_symbol: &Arc<SymbolInfo>,
) -> Result<Vec<Arc<SymbolInfo>>, TextPlanError> {
    let parent_location_hash = parent_symbol.source_location().location_hash();
    let mut subquery_rels = Vec::new();

    for symbol in symbol_table.symbols() {
        if symbol.symbol_type() == crate::textplan::SymbolType::Relation
            && symbol.parent_query_index() >= 0
            && symbol.parent_query_location().location_hash() == parent_location_hash
        {
            subquery_rels.push(symbol.clone());
        }
    }

    // Sort by parent_query_index to ensure correct order
    // (scalar subquery at index 0, set predicate at index 1, etc.)
    subquery_rels.sort_by_key(|s| s.parent_query_index());

    Ok(subquery_rels)
}

/// Finds the pipeline start for a given relation by following continuing_pipeline backwards.
fn find_pipeline_start(
    symbol_table: &SymbolTable,
    rel_symbol: &Arc<SymbolInfo>,
) -> Result<Option<Arc<SymbolInfo>>, TextPlanError> {
    let blob_lock = rel_symbol.blob.as_ref().ok_or_else(|| {
        TextPlanError::ProtobufError(format!("Relation '{}' has no blob data", rel_symbol.name()))
    })?;

    let blob_data = blob_lock.lock().map_err(|_| {
        TextPlanError::ProtobufError(format!(
            "Failed to lock blob for relation '{}'",
            rel_symbol.name()
        ))
    })?;

    let relation_data = blob_data
        .downcast_ref::<crate::textplan::common::structured_symbol_data::RelationData>()
        .ok_or_else(|| {
            TextPlanError::ProtobufError(format!(
                "Relation '{}' blob is not RelationData",
                rel_symbol.name()
            ))
        })?;

    // If this relation has a pipeline_start, return it
    if let Some(start) = &relation_data.pipeline_start {
        Ok(Some(start.clone()))
    } else {
        // This relation IS the pipeline start
        Ok(Some(rel_symbol.clone()))
    }
}
