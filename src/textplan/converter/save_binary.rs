// SPDX-License-Identifier: Apache-2.0

//! Save a Substrait plan to binary format.

use crate::proto::{save_plan_to_binary, Plan, PlanRel};
use crate::textplan::common::error::TextPlanError;
use crate::textplan::common::structured_symbol_data::RelationData;
use crate::textplan::symbol_table::{SymbolInfo, SymbolTable, SymbolType};
use ::substrait::proto::{plan_rel, rel, Rel, RelRoot};
use std::collections::HashSet;
use std::sync::Arc;

/// Creates a Plan protobuf from a symbol table.
///
/// # Arguments
///
/// * `symbol_table` - The symbol table to convert.
///
/// # Returns
///
/// The Plan protobuf representation.
pub fn create_plan_from_symbol_table(symbol_table: &SymbolTable) -> Result<Plan, TextPlanError> {
    use crate::textplan::common::structured_symbol_data::{ExtensionSpaceData, FunctionData};
    use std::collections::HashMap;

    // Create a plan with the appropriate version
    let mut plan = Plan {
        version: Some(::substrait::proto::Version {
            major_number: 0,
            minor_number: 1,
            patch_number: 0,
            git_hash: String::new(),
            producer: "Substrait TextPlan Rust".to_string(),
        }),
        extension_uris: Vec::new(),
        extension_urns: Vec::new(),
        extensions: Vec::new(),
        relations: Vec::new(),
        expected_type_urls: Vec::new(),
        advanced_extensions: None,
        parameter_bindings: Vec::new(),
        type_aliases: Vec::new(),
    };

    // Build extension_uris and extensions from symbol table
    // Collect extension spaces (URIs) and functions
    let mut extension_spaces: HashMap<u32, String> = HashMap::new();
    let mut functions: Vec<(String, Option<u32>, u32)> = Vec::new();

    for symbol in symbol_table.symbols() {
        match symbol.symbol_type() {
            SymbolType::ExtensionSpace => {
                // Extract extension URI and anchor
                if let Some(blob_lock) = &symbol.blob {
                    if let Ok(blob_data) = blob_lock.lock() {
                        if let Some(ext_data) = blob_data.downcast_ref::<ExtensionSpaceData>() {
                            extension_spaces
                                .insert(ext_data.anchor_reference(), symbol.name().to_string());
                        }
                    }
                }
            }
            SymbolType::Function => {
                // Extract function name, extension_uri_reference, and anchor
                if let Some(blob_lock) = &symbol.blob {
                    if let Ok(blob_data) = blob_lock.lock() {
                        if let Some(func_data) = blob_data.downcast_ref::<FunctionData>() {
                            functions.push((
                                func_data.name.clone(),
                                func_data.extension_uri_reference,
                                func_data.anchor,
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Build extension_uris vector from collected extension spaces
    for (anchor, uri) in extension_spaces.iter() {
        plan.extension_uris
            .push(::substrait::proto::extensions::SimpleExtensionUri {
                extension_uri_anchor: *anchor,
                uri: uri.clone(),
            });
    }

    // Build extensions vector from collected functions
    for (name, extension_uri_ref, function_anchor) in functions {
        plan.extensions.push(::substrait::proto::extensions::SimpleExtensionDeclaration {
            mapping_type: Some(
                ::substrait::proto::extensions::simple_extension_declaration::MappingType::ExtensionFunction(
                    ::substrait::proto::extensions::simple_extension_declaration::ExtensionFunction {
                        extension_uri_reference: extension_uri_ref.unwrap_or(0),
                        extension_urn_reference: 0,  // Not used in textplan
                        function_anchor,
                        name,
                    },
                ),
            ),
        });
    }

    println!(
        "Built {} extension URIs and {} extensions",
        plan.extension_uris.len(),
        plan.extensions.len()
    );

    // Find the root symbol if present
    let mut root_names = Vec::new();
    for symbol in symbol_table.symbols() {
        if symbol.symbol_type() == SymbolType::Root {
            // Add roots to the root_names list
            root_names.push(symbol.name().to_string());
        }
    }

    // If we found root names, add a root relation
    if !root_names.is_empty() {
        plan.relations.push(PlanRel {
            rel_type: Some(plan_rel::RelType::Root(RelRoot {
                names: root_names,
                input: None,
            })),
        });
    }

    // Find root relations and recursively build nested trees.
    // Following the C++ implementation in SymbolTablePrinter::outputToBinaryPlan()
    println!(
        "Iterating over symbols, total count: {}",
        symbol_table.symbols().len()
    );
    for symbol in symbol_table.symbols() {
        println!(
            "Checking symbol '{}' of type {:?}",
            symbol.name(),
            symbol.symbol_type()
        );
        if symbol.symbol_type() == SymbolType::Relation {
            // Skip if this is a subquery relation (following C++ logic)
            if symbol.parent_query_index() >= 0 {
                println!(
                    "  Skipping subquery relation '{}' (parent_query_index={})",
                    symbol.name(),
                    symbol.parent_query_index()
                );
                continue;
            }

            // Check if this relation should be output as a top-level PlanRel
            // Output if it's named "root" (special case) or if it's a pipeline terminal
            // that's not nested in another relation's pipeline
            let should_output = symbol.name() == "root";

            if should_output {
                // Check if this is a "root" symbol - wrap in Root plan relation
                if symbol.name() == "root" {
                    // Get the input from new_pipelines[0] and root_names from the root symbol
                    if let Some(blob_lock) = &symbol.blob {
                        if let Ok(blob_data) = blob_lock.lock() {
                            if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                                if !relation_data.new_pipelines.is_empty() {
                                    // Clone input_symbol and root_names to avoid borrow issues
                                    let input_symbol = relation_data.new_pipelines[0].clone();
                                    let root_names = relation_data.root_names.clone();
                                    drop(blob_data); // Drop the lock early

                                    // Get the input's Rel and build the tree
                                    if let Some(input_blob) = &input_symbol.blob {
                                        if let Ok(input_data) = input_blob.lock() {
                                            if let Some(input_rel_data) =
                                                input_data.downcast_ref::<RelationData>()
                                            {
                                                let mut input_rel = input_rel_data.relation.clone();
                                                drop(input_data);

                                                let mut visited = HashSet::new();
                                                add_inputs_to_relation(
                                                    symbol_table,
                                                    &input_symbol,
                                                    &mut input_rel,
                                                    &mut visited,
                                                )?;
                                                // Subqueries are now populated during add_inputs_to_relation

                                                println!(
                                                    "  Root names from root symbol: {:?}",
                                                    root_names
                                                );

                                                // Wrap in Root plan relation
                                                plan.relations.push(PlanRel {
                                                    rel_type: Some(plan_rel::RelType::Root(
                                                        RelRoot {
                                                            input: Some(input_rel),
                                                            names: root_names,
                                                        },
                                                    )),
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // Not a root symbol - add as regular Rel
                    if let Some(blob_lock) = &symbol.blob {
                        if let Ok(blob_data) = blob_lock.lock() {
                            if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                                let mut root_rel = relation_data.relation.clone();
                                // Drop the lock before recursing
                                drop(blob_data);

                                let mut visited = HashSet::new();
                                add_inputs_to_relation(
                                    symbol_table,
                                    symbol,
                                    &mut root_rel,
                                    &mut visited,
                                )?;
                                // Subqueries are now populated during add_inputs_to_relation

                                // Add as a plan relation
                                plan.relations.push(PlanRel {
                                    rel_type: Some(plan_rel::RelType::Rel(root_rel)),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(plan)
}

/// Populates a ReadRel protobuf from symbol table references.
fn populate_read_rel(
    symbol_table: &SymbolTable,
    source_symbol: &Option<Arc<SymbolInfo>>,
    schema_symbol: &Option<Arc<SymbolInfo>>,
    schema_name: &Option<String>,
    read_rel: &mut ::substrait::proto::ReadRel,
) -> Result<(), TextPlanError> {
    // Try to resolve schema symbol by name if not already resolved
    let resolved_schema = if schema_symbol.is_none() && schema_name.is_some() {
        let name = schema_name.as_ref().unwrap();
        // Attempt late binding of schema name to symbol
        symbol_table.lookup_symbol_by_name(name)
    } else {
        schema_symbol.clone()
    };

    // Populate base_schema from schema symbol
    // If baseSchema is already populated in the ReadRel (e.g., from binary), preserve it
    if read_rel.base_schema.is_some() && resolved_schema.is_none() {
        println!("  Preserving existing baseSchema (schema symbol not resolved)");
        // Keep the existing baseSchema
    } else if let Some(schema_sym) = &resolved_schema {
        // Find all SchemaColumn symbols that belong to this schema
        let mut field_names = Vec::new();
        let mut field_types = Vec::new();

        for symbol in symbol_table.symbols() {
            if symbol.symbol_type() == SymbolType::SchemaColumn {
                // Check if this column belongs to our schema
                if let Some(column_schema) = symbol.schema() {
                    if Arc::ptr_eq(&column_schema, schema_sym) {
                        // Add field name
                        field_names.push(symbol.name().to_string());

                        // Add field type from the symbol's blob
                        let field_type = if let Some(blob_lock) = &symbol.blob {
                            if let Ok(blob_data) = blob_lock.lock() {
                                if let Some(proto_type) =
                                    blob_data.downcast_ref::<::substrait::proto::Type>()
                                {
                                    proto_type.clone()
                                } else {
                                    // Fallback to i64 if blob doesn't contain Type
                                    ::substrait::proto::Type {
                                        kind: Some(::substrait::proto::r#type::Kind::I64(
                                            ::substrait::proto::r#type::I64 {
                                                type_variation_reference: 0,
                                                nullability: ::substrait::proto::r#type::Nullability::Required as i32,
                                            },
                                        )),
                                    }
                                }
                            } else {
                                // Fallback to i64 if lock fails
                                ::substrait::proto::Type {
                                    kind: Some(::substrait::proto::r#type::Kind::I64(
                                        ::substrait::proto::r#type::I64 {
                                            type_variation_reference: 0,
                                            nullability:
                                                ::substrait::proto::r#type::Nullability::Required
                                                    as i32,
                                        },
                                    )),
                                }
                            }
                        } else {
                            // Fallback to i64 if no blob
                            ::substrait::proto::Type {
                                kind: Some(::substrait::proto::r#type::Kind::I64(
                                    ::substrait::proto::r#type::I64 {
                                        type_variation_reference: 0,
                                        nullability:
                                            ::substrait::proto::r#type::Nullability::Required as i32,
                                    },
                                )),
                            }
                        };
                        field_types.push(field_type);
                    }
                }
            }
        }

        // Build NamedStruct
        if !field_names.is_empty() {
            read_rel.base_schema = Some(::substrait::proto::NamedStruct {
                names: field_names.clone(),
                r#struct: Some(::substrait::proto::r#type::Struct {
                    types: field_types,
                    type_variation_reference: 0,
                    nullability: ::substrait::proto::r#type::Nullability::Required as i32,
                }),
            });
            println!(
                "  Populated base_schema from schema '{}' with {} fields: {:?}",
                schema_sym.name(),
                field_names.len(),
                field_names
            );
        }
    }

    // Populate namedTable from source symbol
    if let Some(source_sym) = source_symbol {
        // Find SourceDetail symbols that belong to this source
        let mut table_names = Vec::new();
        for symbol in symbol_table.symbols() {
            if symbol.symbol_type() == SymbolType::SourceDetail {
                // Check if this detail belongs to our source
                if let Some(detail_source) = symbol.source() {
                    if Arc::ptr_eq(&detail_source, source_sym) {
                        let name = symbol.name();
                        // Filter out punctuation, keywords, and syntax tokens
                        // Only keep actual table names (uppercase identifiers)
                        if !name.is_empty()
                            && name != "names"  // Filter out the 'names' keyword
                            && name.chars().next().is_some_and(|c| c.is_alphabetic())
                        {
                            table_names.push(name.to_string());
                        }
                    }
                }
            }
        }

        // If we found table names, use them; otherwise fall back to source symbol name
        if table_names.is_empty() {
            table_names.push(source_sym.name().to_string());
        }

        read_rel.read_type = Some(::substrait::proto::read_rel::ReadType::NamedTable(
            ::substrait::proto::read_rel::NamedTable {
                names: table_names.clone(),
                advanced_extension: None,
            },
        ));
        println!(
            "  Populated namedTable with {} tables: {:?}",
            table_names.len(),
            table_names
        );
    }

    // Set common to direct emission (no projection)
    read_rel.common = Some(::substrait::proto::RelCommon {
        emit_kind: Some(::substrait::proto::rel_common::EmitKind::Direct(
            ::substrait::proto::rel_common::Direct {},
        )),
        ..Default::default()
    });

    Ok(())
}

/// Populates a ProjectRel's emit output mappings from symbol table references.
fn populate_project_emit(
    symbol: &Arc<SymbolInfo>,
    project_rel: &mut ::substrait::proto::ProjectRel,
) -> Result<(), TextPlanError> {
    // Get relation data to check for output field references (emits)
    if let Some(blob_lock) = &symbol.blob {
        if let Ok(blob_data) = blob_lock.lock() {
            if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                let emit_count = relation_data.output_field_references.len();

                if emit_count > 0 {
                    // Build output mapping by finding where each output_field_reference appears
                    // IMPORTANT: Must search generated_field_references FIRST (like substrait-cpp),
                    // because field selections create entries in BOTH field_references AND
                    // generated_field_references (same Arc pointer), and we need the generated index.
                    let field_refs_len = relation_data.field_references.len();
                    let mut output_mapping = Vec::new();

                    for output_field in &relation_data.output_field_references {
                        // First search in generated_field_references
                        if let Some(gen_index) = relation_data
                            .generated_field_references
                            .iter()
                            .position(|f| Arc::ptr_eq(f, output_field))
                        {
                            // Found in generated fields - use offset index
                            output_mapping.push((field_refs_len + gen_index) as i32);
                        } else if let Some(field_index) = relation_data
                            .field_references
                            .iter()
                            .position(|f| Arc::ptr_eq(f, output_field))
                        {
                            // Found in field references
                            output_mapping.push(field_index as i32);
                        } else {
                            // Symbol not found - this shouldn't happen, but use a fallback
                            println!(
                                "  WARNING: Output field '{}' not found in field space for '{}'",
                                output_field.name(),
                                symbol.name()
                            );
                            output_mapping.push(0);
                        }
                    }

                    println!(
                        "  Building emit for project '{}': field_refs={}, generated={}, emits={}, mapping={:?}",
                        symbol.name(),
                        relation_data.field_references.len(),
                        relation_data.generated_field_references.len(),
                        emit_count,
                        output_mapping
                    );

                    // Set the RelCommon with emit mapping
                    project_rel.common = Some(::substrait::proto::RelCommon {
                        emit_kind: Some(::substrait::proto::rel_common::EmitKind::Emit(
                            ::substrait::proto::rel_common::Emit { output_mapping },
                        )),
                        ..Default::default()
                    });
                } else {
                    // No emits, use direct emission
                    project_rel.common = Some(::substrait::proto::RelCommon {
                        emit_kind: Some(::substrait::proto::rel_common::EmitKind::Direct(
                            ::substrait::proto::rel_common::Direct {},
                        )),
                        ..Default::default()
                    });
                }
            }
        }
    }

    Ok(())
}

/// Counts the output fields from a relation.
/// This is a simplified version that needs proper implementation.
fn count_relation_output_fields(rel: &::substrait::proto::Rel) -> usize {
    // TODO: Implement proper field counting based on relation type
    // For now, return a placeholder
    if let Some(rel_type) = &rel.rel_type {
        match rel_type {
            ::substrait::proto::rel::RelType::Read(read_rel) => {
                // Count fields from base_schema
                if let Some(base_schema) = &read_rel.base_schema {
                    base_schema.names.len()
                } else {
                    0
                }
            }
            ::substrait::proto::rel::RelType::Cross(cross_rel) => {
                // Cross product combines left and right fields
                let left_count = if let Some(left) = &cross_rel.left {
                    count_relation_output_fields(left)
                } else {
                    0
                };
                let right_count = if let Some(right) = &cross_rel.right {
                    count_relation_output_fields(right)
                } else {
                    0
                };
                left_count + right_count
            }
            ::substrait::proto::rel::RelType::Filter(filter_rel) => {
                // Filter passes through all input fields from its input
                if let Some(input) = &filter_rel.input {
                    count_relation_output_fields(input)
                } else {
                    0
                }
            }
            ::substrait::proto::rel::RelType::Project(proj) => {
                // Project outputs based on emit mapping or expressions
                if let Some(common) = &proj.common {
                    if let Some(emit_kind) = &common.emit_kind {
                        match emit_kind {
                            ::substrait::proto::rel_common::EmitKind::Emit(emit) => {
                                // Emit specifies exactly which fields to output
                                emit.output_mapping.len()
                            }
                            ::substrait::proto::rel_common::EmitKind::Direct(_) => {
                                // Direct emission outputs all input fields plus all expressions
                                let input_count = if let Some(input) = &proj.input {
                                    count_relation_output_fields(input)
                                } else {
                                    0
                                };
                                input_count + proj.expressions.len()
                            }
                        }
                    } else {
                        // No emit specified - output all input fields plus expressions
                        let input_count = if let Some(input) = &proj.input {
                            count_relation_output_fields(input)
                        } else {
                            0
                        };
                        input_count + proj.expressions.len()
                    }
                } else {
                    // No common - output all input fields plus expressions
                    let input_count = if let Some(input) = &proj.input {
                        count_relation_output_fields(input)
                    } else {
                        0
                    };
                    input_count + proj.expressions.len()
                }
            }
            ::substrait::proto::rel::RelType::Aggregate(agg) => {
                // Aggregate outputs grouping keys + measures
                #[allow(deprecated)]
                let grouping_count = agg
                    .groupings
                    .first()
                    .map(|g| g.grouping_expressions.len())
                    .unwrap_or(0);
                let measure_count = agg.measures.len();
                grouping_count + measure_count
            }
            _ => 0, // Other relation types
        }
    } else {
        0
    }
}

/// Helper to get sub_query_pipelines from a symbol.
/// Matches C++ pattern: auto& symbolInfos = symbol.blob->subQueryPipelines;
fn get_subquery_pipelines(symbol: &Arc<SymbolInfo>) -> Vec<Arc<SymbolInfo>> {
    if let Some(blob_lock) = &symbol.blob {
        if let Ok(blob_data) = blob_lock.lock() {
            if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                return relation_data.sub_query_pipelines.clone();
            }
        }
    }
    Vec::new()
}

/// Recursively adds inputs to a relation by following pipeline links.
/// Based on C++ SymbolTablePrinter::addInputsToRelation()
fn add_inputs_to_relation(
    symbol_table: &SymbolTable,
    symbol: &Arc<SymbolInfo>,
    rel: &mut Rel,
    visited: &mut HashSet<*const SymbolInfo>,
) -> Result<(), TextPlanError> {
    println!(
        "add_inputs_to_relation: Entering for symbol '{}'",
        symbol.name()
    );

    // Check for cycles
    let symbol_ptr = Arc::as_ptr(symbol);
    if visited.contains(&symbol_ptr) {
        println!("  Already visited '{}', returning", symbol.name());
        return Ok(()); // Already visited, stop recursion
    }
    visited.insert(symbol_ptr);
    println!("  Marked '{}' as visited", symbol.name());
    // Get the relation data
    let blob_lock = symbol.blob.as_ref().ok_or_else(|| {
        TextPlanError::ProtobufError(format!(
            "Relation symbol '{}' has no blob data",
            symbol.name()
        ))
    })?;

    // Clone ALL the data we need before recursing to avoid holding locks during recursion
    // This includes not just the Arc<SymbolInfo>, but also the Rel protobufs
    println!("  Attempting to lock blob for '{}'", symbol.name());
    let result = {
        let blob_data = blob_lock.lock().map_err(|_| {
            TextPlanError::ProtobufError(format!(
                "Failed to lock blob for relation '{}'",
                symbol.name()
            ))
        })?;

        println!("  Successfully locked blob for '{}'", symbol.name());

        let relation_data = blob_data.downcast_ref::<RelationData>().ok_or_else(|| {
            TextPlanError::ProtobufError(format!(
                "Relation symbol '{}' blob is not RelationData",
                symbol.name()
            ))
        })?;

        // Clone continuing_pipeline symbol and its Rel
        let continuing_pipeline = relation_data.continuing_pipeline.clone();
        let continuing_pipeline_rel = continuing_pipeline.as_ref().and_then(|next| {
            next.blob.as_ref().and_then(|next_blob| {
                next_blob.lock().ok().and_then(|next_data| {
                    next_data
                        .downcast_ref::<RelationData>()
                        .map(|next_rel_data| next_rel_data.relation.clone())
                })
            })
        });

        // Clone new_pipelines symbols and their Rels
        let new_pipelines = relation_data.new_pipelines.clone();
        let new_pipelines_rels: Vec<Option<Rel>> = new_pipelines
            .iter()
            .map(|pipeline_sym| {
                pipeline_sym.blob.as_ref().and_then(|blob| {
                    blob.lock().ok().and_then(|data| {
                        data.downcast_ref::<RelationData>()
                            .map(|rel_data| rel_data.relation.clone())
                    })
                })
            })
            .collect();

        // Clone source and schema symbols for building protobufs
        let source_symbol = relation_data.source.clone();
        let schema_symbol = relation_data.schema.clone();
        let schema_name = relation_data.schema_name.clone();

        println!(
            "  '{}' has continuing_pipeline={:?}, new_pipelines.len={}",
            symbol.name(),
            continuing_pipeline.as_ref().map(|s| s.name()),
            new_pipelines.len()
        );
        (
            continuing_pipeline,
            continuing_pipeline_rel,
            new_pipelines,
            new_pipelines_rels,
            source_symbol,
            schema_symbol,
            schema_name,
        )
    };
    // All locks are dropped here
    let (
        continuing_pipeline,
        continuing_pipeline_rel,
        new_pipelines,
        new_pipelines_rels,
        source_symbol,
        schema_symbol,
        schema_name,
    ) = result;
    println!("  Lock dropped for '{}'", symbol.name());

    // Pattern match on relation type and recursively add inputs
    println!("  Matching on rel_type for '{}'", symbol.name());
    if let Some(rel_type) = &mut rel.rel_type {
        match rel_type {
            // Single-input relations
            rel::RelType::Read(read_rel) => {
                println!("    '{}' is Read (no inputs)", symbol.name());
                // Populate the ReadRel from symbol references
                populate_read_rel(
                    symbol_table,
                    &source_symbol,
                    &schema_symbol,
                    &schema_name,
                    read_rel,
                )?;
                // Read has no inputs
            }
            rel::RelType::Filter(filter_rel) => {
                println!("    '{}' is Filter", symbol.name());

                // Set common to direct emission (filters pass through all fields)
                if filter_rel.common.is_none() {
                    filter_rel.common = Some(::substrait::proto::RelCommon {
                        emit_kind: Some(::substrait::proto::rel_common::EmitKind::Direct(
                            ::substrait::proto::rel_common::Direct {},
                        )),
                        ..Default::default()
                    });
                }

                if let (Some(next), Some(next_rel)) =
                    (&continuing_pipeline, &continuing_pipeline_rel)
                {
                    println!(
                        "      Filter '{}' has continuing_pipeline: '{}'",
                        symbol.name(),
                        next.name()
                    );
                    filter_rel.input = Some(Box::new(next_rel.clone()));
                    if let Some(input) = &mut filter_rel.input {
                        println!(
                            "      Recursing from Filter '{}' to '{}'",
                            symbol.name(),
                            next.name()
                        );
                        add_inputs_to_relation(symbol_table, next, input, visited)?;
                        println!(
                            "      Returned from recursion to '{}' from '{}'",
                            next.name(),
                            symbol.name()
                        );
                    }
                }

                // Populate subqueries in filter condition (matches C++ pattern)
                // Get this symbol's sub_query_pipelines and consume them in expression order
                let subquery_pipelines = get_subquery_pipelines(symbol);
                println!(
                    "      Filter '{}' has {} subquery pipelines",
                    symbol.name(),
                    subquery_pipelines.len()
                );
                let mut consumed_index = 0;
                if let Some(condition) = &mut filter_rel.condition {
                    if !subquery_pipelines.is_empty() {
                        println!("      Populating subqueries for Filter '{}'", symbol.name());
                    }
                    populate_subquery_in_expression(
                        condition,
                        symbol_table,
                        &subquery_pipelines,
                        &mut consumed_index,
                    )?;
                }
            }
            rel::RelType::Project(project_rel) => {
                if let (Some(next), Some(next_rel)) =
                    (&continuing_pipeline, &continuing_pipeline_rel)
                {
                    project_rel.input = Some(Box::new(next_rel.clone()));
                    if let Some(input) = &mut project_rel.input {
                        add_inputs_to_relation(symbol_table, next, input, visited)?;
                    }
                }

                // Populate subqueries in project expressions (matches C++ pattern)
                let subquery_pipelines = get_subquery_pipelines(symbol);
                let mut consumed_index = 0;
                for expr in &mut project_rel.expressions {
                    populate_subquery_in_expression(
                        expr,
                        symbol_table,
                        &subquery_pipelines,
                        &mut consumed_index,
                    )?;
                }

                // Populate emit output mappings if this project has generated field references
                populate_project_emit(symbol, project_rel)?;
            }
            rel::RelType::Aggregate(agg_rel) => {
                println!(
                    "    '{}' is Aggregate with {} measures",
                    symbol.name(),
                    agg_rel.measures.len()
                );

                // Debug: print field indices in measures
                for (i, measure) in agg_rel.measures.iter().enumerate() {
                    if let Some(agg_func) = &measure.measure {
                        for (j, arg) in agg_func.arguments.iter().enumerate() {
                            if let Some(::substrait::proto::function_argument::ArgType::Value(
                                expr,
                            )) = &arg.arg_type
                            {
                                if let Some(::substrait::proto::expression::RexType::Selection(
                                    sel,
                                )) = &expr.rex_type
                                {
                                    if let Some(::substrait::proto::expression::field_reference::ReferenceType::DirectReference(dir_ref)) = &sel.reference_type {
                                        if let Some(::substrait::proto::expression::reference_segment::ReferenceType::StructField(struct_field)) = &dir_ref.reference_type {
                                            println!("      Measure {} arg {} field index: {}", i, j, struct_field.field);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Set common to direct emission (aggregates pass through grouping fields)
                if agg_rel.common.is_none() {
                    agg_rel.common = Some(::substrait::proto::RelCommon {
                        emit_kind: Some(::substrait::proto::rel_common::EmitKind::Direct(
                            ::substrait::proto::rel_common::Direct {},
                        )),
                        ..Default::default()
                    });
                }

                if let (Some(next), Some(next_rel)) =
                    (&continuing_pipeline, &continuing_pipeline_rel)
                {
                    agg_rel.input = Some(Box::new(next_rel.clone()));
                    if let Some(input) = &mut agg_rel.input {
                        add_inputs_to_relation(symbol_table, next, input, visited)?;
                    }
                }
            }
            rel::RelType::Sort(sort_rel) => {
                // Set common to direct emission (sorts pass through all fields)
                if sort_rel.common.is_none() {
                    sort_rel.common = Some(::substrait::proto::RelCommon {
                        emit_kind: Some(::substrait::proto::rel_common::EmitKind::Direct(
                            ::substrait::proto::rel_common::Direct {},
                        )),
                        ..Default::default()
                    });
                }

                if let (Some(next), Some(next_rel)) =
                    (&continuing_pipeline, &continuing_pipeline_rel)
                {
                    sort_rel.input = Some(Box::new(next_rel.clone()));
                    if let Some(input) = &mut sort_rel.input {
                        add_inputs_to_relation(symbol_table, next, input, visited)?;
                    }
                }
            }
            rel::RelType::Fetch(fetch_rel) => {
                println!("    '{}' is Fetch", symbol.name());

                // Set common to direct emission (fetch passes through all fields)
                if fetch_rel.common.is_none() {
                    fetch_rel.common = Some(::substrait::proto::RelCommon {
                        emit_kind: Some(::substrait::proto::rel_common::EmitKind::Direct(
                            ::substrait::proto::rel_common::Direct {},
                        )),
                        ..Default::default()
                    });
                }

                // Fetch is a unary relation (one input from continuing_pipeline)
                if let (Some(next), Some(next_rel)) =
                    (&continuing_pipeline, &continuing_pipeline_rel)
                {
                    println!(
                        "      Fetch '{}' has continuing_pipeline: '{}'",
                        symbol.name(),
                        next.name()
                    );
                    fetch_rel.input = Some(Box::new(next_rel.clone()));
                    if let Some(input) = &mut fetch_rel.input {
                        println!(
                            "      Recursing from Fetch '{}' to '{}'",
                            symbol.name(),
                            next.name()
                        );
                        add_inputs_to_relation(symbol_table, next, input, visited)?;
                        println!(
                            "      Returned from recursion to '{}' from '{}'",
                            next.name(),
                            symbol.name()
                        );
                    }
                }
            }
            rel::RelType::Set(set_rel) => {
                println!("    '{}' is Set", symbol.name());

                // Set common to direct emission
                if set_rel.common.is_none() {
                    set_rel.common = Some(::substrait::proto::RelCommon {
                        emit_kind: Some(::substrait::proto::rel_common::EmitKind::Direct(
                            ::substrait::proto::rel_common::Direct {},
                        )),
                        ..Default::default()
                    });
                }

                // Set relations have multiple inputs in new_pipelines
                for (pipeline_sym, pipeline_rel) in
                    new_pipelines.iter().zip(new_pipelines_rels.iter())
                {
                    if let Some(input_rel) = pipeline_rel {
                        let mut input = input_rel.clone();
                        add_inputs_to_relation(symbol_table, pipeline_sym, &mut input, visited)?;
                        set_rel.inputs.push(input);
                    }
                }
            }
            rel::RelType::HashJoin(hash_join_rel) => {
                println!("    '{}' is HashJoin", symbol.name());

                if new_pipelines.len() >= 2 && new_pipelines_rels.len() >= 2 {
                    // Left input
                    if let Some(left_rel) = &new_pipelines_rels[0] {
                        hash_join_rel.left = Some(Box::new(left_rel.clone()));
                        if let Some(left) = &mut hash_join_rel.left {
                            add_inputs_to_relation(symbol_table, &new_pipelines[0], left, visited)?;
                        }
                    }
                    // Right input
                    if let Some(right_rel) = &new_pipelines_rels[1] {
                        hash_join_rel.right = Some(Box::new(right_rel.clone()));
                        if let Some(right) = &mut hash_join_rel.right {
                            add_inputs_to_relation(
                                symbol_table,
                                &new_pipelines[1],
                                right,
                                visited,
                            )?;
                        }
                    }
                }
            }
            rel::RelType::MergeJoin(merge_join_rel) => {
                println!("    '{}' is MergeJoin", symbol.name());

                if new_pipelines.len() >= 2 && new_pipelines_rels.len() >= 2 {
                    // Left input
                    if let Some(left_rel) = &new_pipelines_rels[0] {
                        merge_join_rel.left = Some(Box::new(left_rel.clone()));
                        if let Some(left) = &mut merge_join_rel.left {
                            add_inputs_to_relation(symbol_table, &new_pipelines[0], left, visited)?;
                        }
                    }
                    // Right input
                    if let Some(right_rel) = &new_pipelines_rels[1] {
                        merge_join_rel.right = Some(Box::new(right_rel.clone()));
                        if let Some(right) = &mut merge_join_rel.right {
                            add_inputs_to_relation(
                                symbol_table,
                                &new_pipelines[1],
                                right,
                                visited,
                            )?;
                        }
                    }
                }
            }
            rel::RelType::ExtensionSingle(ext_single_rel) => {
                println!("    '{}' is ExtensionSingle", symbol.name());

                if let (Some(next), Some(next_rel)) =
                    (&continuing_pipeline, &continuing_pipeline_rel)
                {
                    ext_single_rel.input = Some(Box::new(next_rel.clone()));
                    if let Some(input) = &mut ext_single_rel.input {
                        add_inputs_to_relation(symbol_table, next, input, visited)?;
                    }
                }
            }
            rel::RelType::ExtensionMulti(ext_multi_rel) => {
                println!("    '{}' is ExtensionMulti", symbol.name());

                // Extension multi has multiple inputs in new_pipelines
                for (pipeline_sym, pipeline_rel) in
                    new_pipelines.iter().zip(new_pipelines_rels.iter())
                {
                    if let Some(input_rel) = pipeline_rel {
                        let mut input = input_rel.clone();
                        add_inputs_to_relation(symbol_table, pipeline_sym, &mut input, visited)?;
                        ext_multi_rel.inputs.push(input);
                    }
                }
            }
            rel::RelType::ExtensionLeaf(_ext_leaf_rel) => {
                println!("    '{}' is ExtensionLeaf (no inputs)", symbol.name());
                // Extension leaf has no inputs
            }
            // Binary relations (two inputs)
            rel::RelType::Join(join_rel) => {
                if new_pipelines.len() >= 2 && new_pipelines_rels.len() >= 2 {
                    // Left input
                    if let Some(left_rel) = &new_pipelines_rels[0] {
                        join_rel.left = Some(Box::new(left_rel.clone()));
                        if let Some(left) = &mut join_rel.left {
                            add_inputs_to_relation(symbol_table, &new_pipelines[0], left, visited)?;
                        }
                    }
                    // Right input
                    if let Some(right_rel) = &new_pipelines_rels[1] {
                        join_rel.right = Some(Box::new(right_rel.clone()));
                        if let Some(right) = &mut join_rel.right {
                            add_inputs_to_relation(
                                symbol_table,
                                &new_pipelines[1],
                                right,
                                visited,
                            )?;
                        }
                    }
                }

                // Populate subqueries in join expression (matches C++ pattern)
                let subquery_pipelines = get_subquery_pipelines(symbol);
                let mut consumed_index = 0;
                if let Some(expression) = &mut join_rel.expression {
                    populate_subquery_in_expression(
                        expression,
                        symbol_table,
                        &subquery_pipelines,
                        &mut consumed_index,
                    )?;
                }
            }
            rel::RelType::Cross(cross_rel) => {
                if new_pipelines.len() >= 2 && new_pipelines_rels.len() >= 2 {
                    // Left input
                    if let Some(left_rel) = &new_pipelines_rels[0] {
                        cross_rel.left = Some(Box::new(left_rel.clone()));
                        if let Some(left) = &mut cross_rel.left {
                            add_inputs_to_relation(symbol_table, &new_pipelines[0], left, visited)?;
                        }
                    }
                    // Right input
                    if let Some(right_rel) = &new_pipelines_rels[1] {
                        cross_rel.right = Some(Box::new(right_rel.clone()));
                        if let Some(right) = &mut cross_rel.right {
                            add_inputs_to_relation(
                                symbol_table,
                                &new_pipelines[1],
                                right,
                                visited,
                            )?;
                        }
                    }
                }
            }
            // Other relation types can be added as needed
            _ => {}
        }
    }

    Ok(())
}

/// Saves a Substrait plan to binary format.
///
/// # Arguments
///
/// * `symbol_table` - The symbol table to save.
///
/// # Returns
///
/// The binary representation of the plan.
pub fn save_to_binary(symbol_table: &SymbolTable) -> Result<Vec<u8>, TextPlanError> {
    // Create the plan from the symbol table
    let plan = create_plan_from_symbol_table(symbol_table)?;

    // Note: Subquery relations are now populated inline during plan creation,
    // so we don't need a separate populate_subquery_relations pass

    // Serialize the plan to bytes
    serialize_plan_to_binary(&plan)
}

/// Populates subquery relations in expressions by building them from the symbol tree.
///
/// This function walks through all expressions in the plan and fills in subquery relations
/// that were left as None during parsing. It builds them from the symbol tree using
/// add_inputs_to_relation, ensuring inputs come from pipeline connections rather than
/// copied protobufs.
fn populate_subquery_relations(
    plan: &mut Plan,
    symbol_table: &SymbolTable,
) -> Result<(), TextPlanError> {
    // Find the top-level relation symbols (those that feed the root)
    let top_level_rel_symbols: Vec<Arc<SymbolInfo>> = symbol_table
        .symbols()
        .iter()
        .filter(|s| s.symbol_type() == SymbolType::Relation && s.parent_query_index() < 0)
        .cloned()
        .collect();

    println!(
        "DEBUG: Found {} top-level relation symbols",
        top_level_rel_symbols.len()
    );
    for (i, sym) in top_level_rel_symbols.iter().enumerate() {
        if let Some(blob_lock) = &sym.blob {
            if let Ok(blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                    println!(
                        "DEBUG:   [{}] '{}' has {} sub_query_pipelines",
                        i,
                        sym.name(),
                        relation_data.sub_query_pipelines.len()
                    );
                }
            }
        }
    }

    // Walk through all plan relations
    for (idx, plan_rel) in plan.relations.iter_mut().enumerate() {
        if let Some(plan_rel::RelType::Root(root)) = &mut plan_rel.rel_type {
            println!(
                "DEBUG: Found Root relation, has input: {}",
                root.input.is_some()
            );
            if let Some(input) = &mut root.input {
                // Use the last top-level relation symbol (should be the actual relation that feeds root)
                let symbol_ref = top_level_rel_symbols.last();
                if let Some(sym) = symbol_ref {
                    println!(
                        "DEBUG: Using top-level symbol '{}' for root input",
                        sym.name()
                    );
                }
                populate_subquery_in_rel_with_symbol(input, symbol_table, symbol_ref)?;
            }
        } else if let Some(plan_rel::RelType::Rel(rel)) = &mut plan_rel.rel_type {
            // Use the last top-level relation symbol
            let symbol_ref = top_level_rel_symbols.last();
            if let Some(sym) = symbol_ref {
                println!(
                    "DEBUG: Using top-level symbol '{}' for standalone rel",
                    sym.name()
                );
            }
            populate_subquery_in_rel_with_symbol(rel, symbol_table, symbol_ref)?;
        }
    }

    Ok(())
}

/// Helper that finds the symbol for a relation and populates with its sub_query_pipelines.
/// This function is kept for compatibility but is no longer used since subquery population
/// is now integrated directly into add_inputs_to_relation (matching C++ pattern).
#[allow(dead_code)]
fn populate_subquery_in_rel_with_symbol(
    rel: &mut Rel,
    symbol_table: &SymbolTable,
    known_symbol: Option<&Arc<SymbolInfo>>,
) -> Result<(), TextPlanError> {
    // If we don't have a known symbol, try to find it (for top-level relations)
    // Otherwise use empty pipelines
    let subquery_pipelines = if let Some(sym) = known_symbol {
        // Get sub_query_pipelines from the known symbol
        if let Some(blob_lock) = &sym.blob {
            if let Ok(blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                    relation_data.sub_query_pipelines.clone()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let mut consumed_index = 0;
    populate_subquery_in_rel_impl(rel, symbol_table, &subquery_pipelines, &mut consumed_index)
}

/// Recursively populates subquery relations in a Rel and its nested expressions.
fn populate_subquery_in_rel(
    rel: &mut Rel,
    symbol_table: &SymbolTable,
) -> Result<(), TextPlanError> {
    // Start with empty subquery pipelines - will be populated from relation symbols
    let subquery_pipelines: Vec<Arc<SymbolInfo>> = Vec::new();
    let mut consumed_index = 0;
    populate_subquery_in_rel_impl(rel, symbol_table, &subquery_pipelines, &mut consumed_index)
}

/// Internal implementation that uses indexed access to subquery pipelines.
fn populate_subquery_in_rel_impl(
    rel: &mut Rel,
    symbol_table: &SymbolTable,
    subquery_pipelines: &[Arc<SymbolInfo>],
    consumed_index: &mut usize,
) -> Result<(), TextPlanError> {
    use substrait::proto::rel::RelType;

    let rel_type_name = match &rel.rel_type {
        Some(RelType::Filter(_)) => "Filter",
        Some(RelType::Project(_)) => "Project",
        Some(RelType::Join(_)) => "Join",
        Some(RelType::Sort(_)) => "Sort",
        Some(RelType::Cross(_)) => "Cross",
        Some(RelType::Read(_)) => "Read",
        Some(RelType::Aggregate(_)) => "Aggregate",
        Some(RelType::Fetch(_)) => "Fetch",
        None => "None",
        Some(other) => {
            eprintln!(
                "WARNING: Encountered unhandled relation type in populate_subquery_in_rel: {:?}",
                other
            );
            "Other"
        }
    };
    println!(
        "    DEBUG populate_subquery_in_rel: Processing {} relation",
        rel_type_name
    );

    match &mut rel.rel_type {
        Some(RelType::Filter(filter)) => {
            println!(
                "    DEBUG: Processing filter relation, has condition: {}",
                filter.condition.is_some()
            );
            if let Some(condition) = &mut filter.condition {
                populate_subquery_in_expression(
                    condition,
                    symbol_table,
                    subquery_pipelines,
                    consumed_index,
                )?;
            }
            if let Some(input) = &mut filter.input {
                populate_subquery_in_rel_impl(
                    input,
                    symbol_table,
                    subquery_pipelines,
                    consumed_index,
                )?;
            }
        }
        Some(RelType::Project(project)) => {
            for expr in &mut project.expressions {
                populate_subquery_in_expression(
                    expr,
                    symbol_table,
                    subquery_pipelines,
                    consumed_index,
                )?;
            }
            if let Some(input) = &mut project.input {
                populate_subquery_in_rel_impl(
                    input,
                    symbol_table,
                    subquery_pipelines,
                    consumed_index,
                )?;
            }
        }
        Some(RelType::Join(join)) => {
            if let Some(expr) = &mut join.expression {
                populate_subquery_in_expression(
                    expr,
                    symbol_table,
                    subquery_pipelines,
                    consumed_index,
                )?;
            }
            if let Some(left) = &mut join.left {
                populate_subquery_in_rel_impl(
                    left,
                    symbol_table,
                    subquery_pipelines,
                    consumed_index,
                )?;
            }
            if let Some(right) = &mut join.right {
                populate_subquery_in_rel_impl(
                    right,
                    symbol_table,
                    subquery_pipelines,
                    consumed_index,
                )?;
            }
        }
        Some(RelType::Read(_)) => {
            // Read has no inputs or expressions with subqueries
        }
        Some(RelType::Sort(sort)) => {
            // Sort has no expressions with subqueries, but recurse into input
            if let Some(input) = &mut sort.input {
                populate_subquery_in_rel_impl(
                    input,
                    symbol_table,
                    subquery_pipelines,
                    consumed_index,
                )?;
            }
        }
        Some(RelType::Cross(cross)) => {
            // Cross has no condition, but recurse into left and right
            if let Some(left) = &mut cross.left {
                populate_subquery_in_rel_impl(
                    left,
                    symbol_table,
                    subquery_pipelines,
                    consumed_index,
                )?;
            }
            if let Some(right) = &mut cross.right {
                populate_subquery_in_rel_impl(
                    right,
                    symbol_table,
                    subquery_pipelines,
                    consumed_index,
                )?;
            }
        }
        Some(RelType::Aggregate(agg)) => {
            // Aggregate can have expressions in measures, but no subqueries typically
            // Recurse into input
            if let Some(input) = &mut agg.input {
                populate_subquery_in_rel_impl(
                    input,
                    symbol_table,
                    subquery_pipelines,
                    consumed_index,
                )?;
            }
        }
        Some(RelType::Fetch(fetch)) => {
            // Fetch has no expressions, recurse into input
            if let Some(input) = &mut fetch.input {
                populate_subquery_in_rel_impl(
                    input,
                    symbol_table,
                    subquery_pipelines,
                    consumed_index,
                )?;
            }
        }
        // Add other relation types as needed
        _ => {}
    }

    Ok(())
}

/// Populates subquery relations in an expression.
fn populate_subquery_in_expression(
    expr: &mut substrait::proto::Expression,
    symbol_table: &SymbolTable,
    subquery_pipelines: &[Arc<SymbolInfo>],
    consumed_index: &mut usize,
) -> Result<(), TextPlanError> {
    use substrait::proto::expression::{subquery::SubqueryType, RexType};

    let rex_type_name = match &expr.rex_type {
        Some(RexType::Subquery(_)) => "Subquery",
        Some(RexType::ScalarFunction(_)) => "ScalarFunction",
        Some(RexType::Cast(_)) => "Cast",
        Some(RexType::Literal(_)) => "Literal",
        Some(RexType::Selection(_)) => "Selection",
        _ => "Other",
    };
    println!(
        "      DEBUG populate_subquery_in_expression: expr rex_type = {}",
        rex_type_name
    );

    match &mut expr.rex_type {
        Some(RexType::Subquery(subquery)) => {
            // Check if the subquery relation needs to be populated
            match &mut subquery.subquery_type {
                Some(SubqueryType::SetComparison(set_comp)) => {
                    println!(
                        "      DEBUG: Checking SetComparison, consumed_index={}, pipelines.len()={}",
                        *consumed_index, subquery_pipelines.len()
                    );

                    // Use indexed access like C++ does: auto subquerySymbol = symbolInfos[(*consumedPipelines)++];
                    if *consumed_index < subquery_pipelines.len() {
                        let symbol = &subquery_pipelines[*consumed_index];
                        *consumed_index += 1;

                        println!(
                            "        Building SetComparison subquery relation '{}' at index {}",
                            symbol.name(),
                            *consumed_index - 1
                        );

                        // Build the Rel from the symbol tree
                        if let Some(blob_lock) = &symbol.blob {
                            if let Ok(blob_data) = blob_lock.lock() {
                                if let Some(relation_data) =
                                    blob_data.downcast_ref::<RelationData>()
                                {
                                    let mut subquery_rel = relation_data.relation.clone();

                                    // Get this subquery's sub_query_pipelines for nested subqueries
                                    let nested_pipelines =
                                        relation_data.sub_query_pipelines.clone();
                                    drop(blob_data); // Drop lock before recursing

                                    // Populate inputs from symbol tree
                                    let mut visited = HashSet::new();
                                    add_inputs_to_relation(
                                        symbol_table,
                                        symbol,
                                        &mut subquery_rel,
                                        &mut visited,
                                    )?;

                                    // Recurse to populate any nested subqueries using this subquery's pipelines
                                    let mut nested_index = 0;
                                    populate_subquery_in_rel_impl(
                                        &mut subquery_rel,
                                        symbol_table,
                                        &nested_pipelines,
                                        &mut nested_index,
                                    )?;

                                    set_comp.right = Some(Box::new(subquery_rel));
                                    println!(
                                        "        Successfully populated SetComparison subquery relation '{}'",
                                        symbol.name()
                                    );
                                }
                            }
                        }
                    }

                    // Recursively handle left expression
                    if let Some(left) = &mut set_comp.left {
                        populate_subquery_in_expression(
                            left,
                            symbol_table,
                            subquery_pipelines,
                            consumed_index,
                        )?;
                    }
                }
                Some(SubqueryType::Scalar(scalar)) => {
                    println!(
                        "      DEBUG: Checking Scalar subquery, consumed_index={}, pipelines.len()={}",
                        *consumed_index, subquery_pipelines.len()
                    );

                    // Use indexed access like C++ does: auto subquerySymbol = symbolInfos[(*consumedPipelines)++];
                    if *consumed_index < subquery_pipelines.len() {
                        let symbol = &subquery_pipelines[*consumed_index];
                        *consumed_index += 1;

                        println!(
                            "        Building Scalar subquery relation '{}' at index {}",
                            symbol.name(),
                            *consumed_index - 1
                        );

                        // Build the Rel from the symbol tree
                        if let Some(blob_lock) = &symbol.blob {
                            if let Ok(blob_data) = blob_lock.lock() {
                                if let Some(relation_data) =
                                    blob_data.downcast_ref::<RelationData>()
                                {
                                    let mut subquery_rel = relation_data.relation.clone();

                                    // Get this subquery's sub_query_pipelines for nested subqueries
                                    let nested_pipelines =
                                        relation_data.sub_query_pipelines.clone();
                                    drop(blob_data); // Drop lock before recursing

                                    // Populate inputs from symbol tree
                                    let mut visited = HashSet::new();
                                    add_inputs_to_relation(
                                        symbol_table,
                                        symbol,
                                        &mut subquery_rel,
                                        &mut visited,
                                    )?;

                                    // Recurse to populate any nested subqueries using this subquery's pipelines
                                    let mut nested_index = 0;
                                    populate_subquery_in_rel_impl(
                                        &mut subquery_rel,
                                        symbol_table,
                                        &nested_pipelines,
                                        &mut nested_index,
                                    )?;

                                    scalar.input = Some(Box::new(subquery_rel));
                                    println!(
                                        "        Successfully populated Scalar subquery relation '{}'",
                                        symbol.name()
                                    );
                                }
                            }
                        }
                    }
                }
                Some(SubqueryType::InPredicate(in_pred)) => {
                    println!(
                        "      DEBUG: Checking InPredicate, consumed_index={}, pipelines.len()={}",
                        *consumed_index,
                        subquery_pipelines.len()
                    );

                    // Use indexed access like C++ does: auto subquerySymbol = symbolInfos[(*consumedPipelines)++];
                    if *consumed_index < subquery_pipelines.len() {
                        let symbol = &subquery_pipelines[*consumed_index];
                        *consumed_index += 1;

                        println!(
                            "        Building InPredicate subquery relation '{}' at index {}",
                            symbol.name(),
                            *consumed_index - 1
                        );

                        // Build the Rel from the symbol tree
                        if let Some(blob_lock) = &symbol.blob {
                            if let Ok(blob_data) = blob_lock.lock() {
                                if let Some(relation_data) =
                                    blob_data.downcast_ref::<RelationData>()
                                {
                                    let mut subquery_rel = relation_data.relation.clone();

                                    // Get this subquery's sub_query_pipelines for nested subqueries
                                    let nested_pipelines =
                                        relation_data.sub_query_pipelines.clone();
                                    drop(blob_data); // Drop lock before recursing

                                    // Populate inputs from symbol tree
                                    let mut visited = HashSet::new();
                                    add_inputs_to_relation(
                                        symbol_table,
                                        symbol,
                                        &mut subquery_rel,
                                        &mut visited,
                                    )?;

                                    // Recurse to populate any nested subqueries using this subquery's pipelines
                                    let mut nested_index = 0;
                                    populate_subquery_in_rel_impl(
                                        &mut subquery_rel,
                                        symbol_table,
                                        &nested_pipelines,
                                        &mut nested_index,
                                    )?;

                                    in_pred.haystack = Some(Box::new(subquery_rel));
                                    println!(
                                        "        Successfully populated InPredicate subquery relation '{}'",
                                        symbol.name()
                                    );
                                }
                            }
                        }
                    }

                    // Recursively handle needle expressions
                    for needle in &mut in_pred.needles {
                        populate_subquery_in_expression(
                            needle,
                            symbol_table,
                            subquery_pipelines,
                            consumed_index,
                        )?;
                    }
                }
                Some(SubqueryType::SetPredicate(set_pred)) => {
                    println!(
                        "      DEBUG: Checking SetPredicate, consumed_index={}, pipelines.len()={}",
                        *consumed_index,
                        subquery_pipelines.len()
                    );

                    // Use indexed access like C++ does: auto subquerySymbol = symbolInfos[(*consumedPipelines)++];
                    if *consumed_index < subquery_pipelines.len() {
                        let symbol = &subquery_pipelines[*consumed_index];
                        *consumed_index += 1;

                        println!(
                            "        Building SetPredicate subquery relation '{}' at index {}",
                            symbol.name(),
                            *consumed_index - 1
                        );

                        // Build the Rel from the symbol tree
                        if let Some(blob_lock) = &symbol.blob {
                            if let Ok(blob_data) = blob_lock.lock() {
                                if let Some(relation_data) =
                                    blob_data.downcast_ref::<RelationData>()
                                {
                                    let mut subquery_rel = relation_data.relation.clone();

                                    // Get this subquery's sub_query_pipelines for nested subqueries
                                    let nested_pipelines =
                                        relation_data.sub_query_pipelines.clone();
                                    drop(blob_data); // Drop lock before recursing

                                    // Populate inputs from symbol tree
                                    let mut visited = HashSet::new();
                                    add_inputs_to_relation(
                                        symbol_table,
                                        symbol,
                                        &mut subquery_rel,
                                        &mut visited,
                                    )?;

                                    // Recurse to populate any nested subqueries using this subquery's pipelines
                                    let mut nested_index = 0;
                                    populate_subquery_in_rel_impl(
                                        &mut subquery_rel,
                                        symbol_table,
                                        &nested_pipelines,
                                        &mut nested_index,
                                    )?;

                                    set_pred.tuples = Some(Box::new(subquery_rel));
                                    println!(
                                        "        Successfully populated SetPredicate subquery relation '{}'",
                                        symbol.name()
                                    );
                                }
                            }
                        }
                    }
                }
                None => {}
            }
        }
        Some(RexType::ScalarFunction(func)) => {
            // Recursively handle function arguments
            for arg in &mut func.arguments {
                if let Some(substrait::proto::function_argument::ArgType::Value(inner_expr)) =
                    &mut arg.arg_type
                {
                    populate_subquery_in_expression(
                        inner_expr,
                        symbol_table,
                        subquery_pipelines,
                        consumed_index,
                    )?;
                }
            }
        }
        Some(RexType::Cast(cast)) => {
            // Recurse into the cast input expression
            if let Some(input) = &mut cast.input {
                populate_subquery_in_expression(
                    input,
                    symbol_table,
                    subquery_pipelines,
                    consumed_index,
                )?;
            }
        }
        _ => {}
    }

    Ok(())
}

/// Serializes a Plan to binary protobuf using prost.
///
/// # Arguments
///
/// * `plan` - The plan to serialize.
///
/// # Returns
///
/// The binary protobuf representation of the plan.
fn serialize_plan_to_binary(plan: &Plan) -> Result<Vec<u8>, TextPlanError> {
    // Use the existing function if available, otherwise implement with prost
    save_plan_to_binary(plan)
}
