// SPDX-License-Identifier: Apache-2.0

//! Main plan visitor for processing top-level plan structures.

use std::any::Any;
use std::sync::{Arc, Mutex};

use antlr_rust::parser_rule_context::ParserRuleContext;
use antlr_rust::rule_context::RuleContext;
use antlr_rust::token::{GenericToken, Token};
use antlr_rust::tree::{ParseTree, ParseTreeVisitor};
use antlr_rust::TidExt;

use crate::textplan::common::structured_symbol_data::RelationData;
use crate::textplan::parser::antlr::substraitplanparser::*;
use crate::textplan::parser::antlr::substraitplanparservisitor::SubstraitPlanParserVisitor;
use crate::textplan::parser::error_listener::ErrorListener;
use crate::textplan::symbol_table::{RelationType, SymbolInfo, SymbolTable, SymbolType};
use ::substrait::proto::{rel::RelType, Rel};

use super::{extract_from_string, token_to_location, PlanVisitor, TypeVisitor};

/// The PlanVisitor processes the top-level plan structure.
///
/// This visitor is the second phase in the multiphase parsing approach,
/// building on the TypeVisitor to handle plan-level structures.
pub struct MainPlanVisitor<'input> {
    type_visitor: TypeVisitor<'input>,
    current_relation_scope: Option<Arc<SymbolInfo>>, // Use actual SymbolInfo
    current_source_scope: Option<Arc<SymbolInfo>>,   // Track current source being processed
    current_extension_space: Option<Arc<SymbolInfo>>, // Track current extension space
    num_spaces_seen: i32,
    num_functions_seen: i32,
}

impl<'input> MainPlanVisitor<'input> {
    /// Creates a new MainPlanVisitor.
    pub fn new(symbol_table: SymbolTable, error_listener: Arc<ErrorListener>) -> Self {
        Self {
            type_visitor: TypeVisitor::new(symbol_table, error_listener.clone()),
            current_relation_scope: None,
            current_source_scope: None,
            current_extension_space: None,
            num_spaces_seen: 0,
            num_functions_seen: 0,
        }
    }

    /// Gets the current relation scope, if any.
    pub fn current_relation_scope(&self) -> Option<&Arc<SymbolInfo>> {
        self.current_relation_scope.as_ref()
    }

    /// Sets the current relation scope.
    pub fn set_current_relation_scope(&mut self, scope: Option<Arc<SymbolInfo>>) {
        self.current_relation_scope = scope;
    }

    /// Gets the current source scope, if any.
    pub fn current_source_scope(&self) -> Option<&Arc<SymbolInfo>> {
        self.current_source_scope.as_ref()
    }

    /// Sets the current source scope.
    pub fn set_current_source_scope(&mut self, scope: Option<Arc<SymbolInfo>>) {
        self.current_source_scope = scope;
    }

    /// Gets the error listener for this visitor.
    pub fn get_error_listener(&self) -> Arc<ErrorListener> {
        self.type_visitor.error_listener()
    }

    /// Gets the symbol table for this visitor.
    pub fn get_symbol_table(&self) -> SymbolTable {
        self.type_visitor.symbol_table()
    }

    /// Adds an error message to the error listener.
    pub fn add_error<'a>(
        &self,
        token: &impl std::ops::Deref<Target = GenericToken<std::borrow::Cow<'a, str>>>,
        message: &str,
    ) {
        self.type_visitor.add_error(token, message);
    }

    /// Process an extension space and add it to the symbol table.
    fn process_extension_space(
        &mut self,
        ctx: &ExtensionspaceContext<'input>,
    ) -> Option<Arc<SymbolInfo>> {
        // Get the name of the extension space from the URI token if present
        // If no URI, create an unnamed extension space
        let (name, location) = if let Some(uri_token) = ctx.URI() {
            let name = uri_token.get_text().trim().to_string();
            let location = token_to_location(&uri_token.symbol);
            (name, location)
        } else {
            // No URI provided, use a default name and location from the context
            let token = ctx.start();
            let location = token_to_location(&token);
            ("unnamed_extension_space".to_string(), location)
        };

        // Assign an anchor for this extension space (incrementing counter)
        let anchor = self.num_spaces_seen as u32;

        // Create ExtensionSpaceData blob
        let extension_space_data =
            crate::textplan::common::structured_symbol_data::ExtensionSpaceData::new(anchor);
        let blob = Some(Arc::new(std::sync::Mutex::new(extension_space_data))
            as Arc<std::sync::Mutex<dyn std::any::Any + Send + Sync>>);

        // Define the extension space in the symbol table
        let symbol = self.type_visitor.symbol_table_mut().define_symbol(
            name,
            location,
            SymbolType::ExtensionSpace,
            None, // subtype
            blob, // blob
        );

        println!(
            "  Defined extension space '{}' with anchor {}",
            symbol.name(),
            anchor
        );

        Some(symbol)
    }

    /// Process a function definition and add it to the symbol table.
    fn process_function(&mut self, ctx: &FunctionContext<'input>) -> Option<Arc<SymbolInfo>> {
        // Get the function signature (e.g., "multiply:dec_dec")
        let full_name = ctx.name()?.get_text();

        // Get the alias (the name after "AS", e.g., "multiply")
        // If no alias, use the function name before the colon
        let alias = if let Some(id_ctx) = ctx.id() {
            id_ctx.get_text()
        } else {
            // No alias - use function name before colon
            full_name
                .split(':')
                .next()
                .unwrap_or(&full_name)
                .to_string()
        };

        // Create a location from the context's start token
        let token = ctx.start();
        let location = token_to_location(&token);

        // Assign an anchor for this function (incrementing counter)
        let anchor = self.num_functions_seen as u32;

        // Get extension_uri_reference from current extension space
        let extension_uri_reference = if let Some(ext_space) = &self.current_extension_space {
            // Get the anchor from the extension space blob
            if let Some(blob_lock) = &ext_space.blob {
                if let Ok(blob_data) = blob_lock.lock() {
                    blob_data.downcast_ref::<crate::textplan::common::structured_symbol_data::ExtensionSpaceData>().map(|ext_data| ext_data.anchor_reference())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Create FunctionData blob
        let function_data = crate::textplan::common::structured_symbol_data::FunctionData::new(
            full_name.clone(),
            extension_uri_reference,
            anchor,
        );
        let blob = Some(Arc::new(std::sync::Mutex::new(function_data))
            as Arc<std::sync::Mutex<dyn std::any::Any + Send + Sync>>);

        // Define the function in the symbol table with the alias as the name
        let symbol = self.type_visitor.symbol_table_mut().define_symbol(
            alias,
            location,
            SymbolType::Function,
            None, // subtype
            blob, // blob
        );

        println!(
            "  Defined function '{}' (alias '{}') with anchor {}, extension_uri_ref {:?}",
            full_name,
            symbol.name(),
            anchor,
            extension_uri_reference
        );

        Some(symbol)
    }

    /// Process a source definition and add it to the symbol table.
    fn process_source_definition(
        &mut self,
        ctx: &Source_definitionContext<'input>,
    ) -> Option<Arc<SymbolInfo>> {
        use crate::textplan::parser::antlr::substraitplanparser::Read_typeContextAll;

        // Get the read_type which can be NamedTable, LocalFiles, VirtualTable, or ExtensionTable
        let read_type_ctx = ctx.read_type()?;

        // Match on the specific read type variant
        match read_type_ctx.as_ref() {
            Read_typeContextAll::NamedTableContext(named_table_ctx) => {
                // Get the id (source name) from the named table context
                let name = named_table_ctx.id()?.get_text();

                // Create a location from the context's start token
                let token = ctx.start();
                let location = token_to_location(&token);

                // Define the source in the symbol table
                let symbol = self.type_visitor.symbol_table_mut().define_symbol(
                    name,
                    location,
                    SymbolType::Source,
                    None,
                    None,
                );

                Some(symbol)
            }
            _ => {
                // For other source types, we don't process them yet
                None
            }
        }
    }

    /// Process named table details and add string symbols to the symbol table.
    fn process_named_table_detail(
        &mut self,
        ctx: &Named_table_detailContext<'input>,
        parent_source: &Arc<SymbolInfo>,
    ) -> Option<()> {
        // Get all STRING tokens from the context
        let strings = ctx.STRING_all();

        for string_token in strings {
            // Get the text of the string token
            let text = string_token.get_text();

            // Extract the string content (remove quotes)
            let name = extract_from_string(&text);

            // Create a location from the string token
            let location = token_to_location(&string_token.symbol);

            // Define the source detail in the symbol table
            let symbol = self.type_visitor.symbol_table_mut().define_symbol(
                name,
                location,
                SymbolType::SourceDetail,
                None,
                None,
            );

            // Set the source as the parent (similar to SchemaColumn→Schema)
            symbol.set_source(parent_source.clone());
        }

        Some(())
    }

    /// Add input fields from continuing pipeline and new pipelines to the current relation's field_references.
    /// For READ relations, populates from the schema. For other relations, populates from input pipelines.
    /// This recursively populates upstream relations first to ensure fields are available.
    fn add_input_fields_to_schema(&mut self, relation_symbol: &Arc<SymbolInfo>) {
        println!(
            "    add_input_fields_to_schema called for '{}'",
            relation_symbol.name()
        );

        // Check if already populated (early return to avoid unnecessary work)
        if let Some(blob_lock) = &relation_symbol.blob {
            if let Ok(blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                    if !relation_data.field_references.is_empty() {
                        println!(
                            "      '{}' already has {} field_references, skipping",
                            relation_symbol.name(),
                            relation_data.field_references.len()
                        );
                        return; // Already populated
                    }
                }
            }
        }

        // Recursively populate upstream relations first
        if let Some(blob_lock) = &relation_symbol.blob {
            if let Ok(blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                    // Collect upstream relations to populate (need to drop lock before recursing)
                    let mut upstreams = Vec::new();
                    if let Some(cont) = &relation_data.continuing_pipeline {
                        upstreams.push(cont.clone());
                    }
                    for pipe in &relation_data.new_pipelines {
                        upstreams.push(pipe.clone());
                    }
                    drop(blob_data);

                    // Recursively populate upstreams
                    for upstream in upstreams {
                        self.add_input_fields_to_schema(&upstream);
                    }
                }
            }
        }

        // Now populate this relation's field_references from its (now-populated) upstreams
        if let Some(blob_lock) = &relation_symbol.blob {
            if let Ok(mut blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_mut::<RelationData>() {
                    // Check if this is a READ relation
                    if let Some(RelType::Read(_)) = &relation_data.relation.rel_type {
                        println!(
                            "      '{}' is READ, populating from schema",
                            relation_symbol.name()
                        );
                        // For READ relations, populate field_references from the schema
                        if let Some(schema_arc) = &relation_data.schema {
                            println!("        Schema: '{}'", schema_arc.name());
                            for symbol in self.symbol_table().symbols() {
                                if symbol.symbol_type() == SymbolType::SchemaColumn {
                                    if let Some(symbol_schema) = symbol.schema() {
                                        if Arc::ptr_eq(&symbol_schema, schema_arc) {
                                            println!("          Adding field: '{}'", symbol.name());
                                            relation_data.field_references.push(symbol.clone());
                                        }
                                    }
                                }
                            }
                            println!(
                                "        Total fields added: {}",
                                relation_data.field_references.len()
                            );
                        } else {
                            println!("        No schema found!");
                        }
                        return;
                    }

                    // For non-READ relations, add fields from continuing pipeline
                    if let Some(continuing_pipeline) = &relation_data.continuing_pipeline {
                        if let Some(cont_blob_lock) = &continuing_pipeline.blob {
                            if let Ok(cont_blob_data) = cont_blob_lock.lock() {
                                if let Some(cont_relation_data) =
                                    cont_blob_data.downcast_ref::<RelationData>()
                                {
                                    println!("      Upstream '{}': output_field_refs={}, field_refs={}, generated={}",
                                        continuing_pipeline.name(),
                                        cont_relation_data.output_field_references.len(),
                                        cont_relation_data.field_references.len(),
                                        cont_relation_data.generated_field_references.len());

                                    if !cont_relation_data.output_field_references.is_empty() {
                                        println!(
                                            "        Using output_field_references ({} fields)",
                                            cont_relation_data.output_field_references.len()
                                        );
                                        for field in &cont_relation_data.output_field_references {
                                            relation_data.field_references.push(field.clone());
                                        }
                                    } else {
                                        println!("        Using field_references + generated ({} + {} fields)",
                                            cont_relation_data.field_references.len(),
                                            cont_relation_data.generated_field_references.len());
                                        for field in &cont_relation_data.field_references {
                                            relation_data.field_references.push(field.clone());
                                        }
                                        for field in &cont_relation_data.generated_field_references
                                        {
                                            relation_data.field_references.push(field.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Add fields from new pipelines (e.g., for joins)
                    for pipeline in &relation_data.new_pipelines.clone() {
                        if let Some(pipe_blob_lock) = &pipeline.blob {
                            if let Ok(pipe_blob_data) = pipe_blob_lock.lock() {
                                if let Some(pipe_relation_data) =
                                    pipe_blob_data.downcast_ref::<RelationData>()
                                {
                                    if !pipe_relation_data.output_field_references.is_empty() {
                                        for field in &pipe_relation_data.output_field_references {
                                            relation_data.field_references.push(field.clone());
                                        }
                                    } else {
                                        for field in &pipe_relation_data.field_references {
                                            relation_data.field_references.push(field.clone());
                                        }
                                        for field in &pipe_relation_data.generated_field_references
                                        {
                                            relation_data.field_references.push(field.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Process a relation and add it to the symbol table.
    fn process_relation(&mut self, ctx: &RelationContext<'input>) -> Option<Arc<SymbolInfo>> {
        // Get the name from the relation_ref's first id
        let relation_ref = ctx.relation_ref()?;
        let name = relation_ref.id(0)?.get_text();

        // Create a location from the context's start token
        // Get the start token directly - it's not an Option
        let token = ctx.start();
        let location = token_to_location(&token);

        // Determine the relation type and create corresponding Rel protobuf
        let (relation_type, rel_protobuf) = if let Some(relation_type_ctx) = ctx.relation_type() {
            let type_text = relation_type_ctx.get_text().to_lowercase();
            match type_text.as_str() {
                "read" => (
                    RelationType::Read,
                    Rel {
                        rel_type: Some(RelType::Read(Box::default())),
                    },
                ),
                "project" => (
                    RelationType::Project,
                    Rel {
                        rel_type: Some(RelType::Project(Box::default())),
                    },
                ),
                "join" => (
                    RelationType::Join,
                    Rel {
                        rel_type: Some(RelType::Join(Box::new(::substrait::proto::JoinRel {
                            common: Some(::substrait::proto::RelCommon {
                                emit_kind: Some(::substrait::proto::rel_common::EmitKind::Direct(
                                    ::substrait::proto::rel_common::Direct {},
                                )),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }))),
                    },
                ),
                "cross" => (
                    RelationType::Cross,
                    Rel {
                        rel_type: Some(RelType::Cross(Box::new(::substrait::proto::CrossRel {
                            common: Some(::substrait::proto::RelCommon {
                                emit_kind: Some(::substrait::proto::rel_common::EmitKind::Direct(
                                    ::substrait::proto::rel_common::Direct {},
                                )),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }))),
                    },
                ),
                "fetch" => (
                    RelationType::Fetch,
                    Rel {
                        rel_type: Some(RelType::Fetch(Box::default())),
                    },
                ),
                "aggregate" => {
                    let mut agg_rel = ::substrait::proto::AggregateRel::default();
                    // Add empty grouping (required if no measures)
                    #[allow(deprecated)]
                    {
                        agg_rel
                            .groupings
                            .push(::substrait::proto::aggregate_rel::Grouping {
                                grouping_expressions: Vec::new(),
                                expression_references: Vec::new(),
                            });
                    }
                    (
                        RelationType::Aggregate,
                        Rel {
                            rel_type: Some(RelType::Aggregate(Box::new(agg_rel))),
                        },
                    )
                }
                "sort" => (
                    RelationType::Sort,
                    Rel {
                        rel_type: Some(RelType::Sort(Box::default())),
                    },
                ),
                "filter" => (
                    RelationType::Filter,
                    Rel {
                        rel_type: Some(RelType::Filter(Box::default())),
                    },
                ),
                "set" => (
                    RelationType::Set,
                    Rel {
                        rel_type: Some(RelType::Set(::substrait::proto::SetRel::default())),
                    },
                ),
                "hash_join" => (
                    RelationType::HashJoin,
                    Rel {
                        rel_type: Some(RelType::HashJoin(Box::new(
                            ::substrait::proto::HashJoinRel {
                                common: Some(::substrait::proto::RelCommon {
                                    emit_kind: Some(
                                        ::substrait::proto::rel_common::EmitKind::Direct(
                                            ::substrait::proto::rel_common::Direct {},
                                        ),
                                    ),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                        ))),
                    },
                ),
                "merge_join" => (
                    RelationType::MergeJoin,
                    Rel {
                        rel_type: Some(RelType::MergeJoin(Box::new(
                            ::substrait::proto::MergeJoinRel {
                                common: Some(::substrait::proto::RelCommon {
                                    emit_kind: Some(
                                        ::substrait::proto::rel_common::EmitKind::Direct(
                                            ::substrait::proto::rel_common::Direct {},
                                        ),
                                    ),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                        ))),
                    },
                ),
                "exchange" => (
                    RelationType::Exchange,
                    Rel {
                        rel_type: Some(RelType::Exchange(Box::default())),
                    },
                ),
                "ddl" => (
                    RelationType::Ddl,
                    Rel {
                        rel_type: Some(RelType::Ddl(Box::default())),
                    },
                ),
                "write" => (
                    RelationType::Write,
                    Rel {
                        rel_type: Some(RelType::Write(Box::default())),
                    },
                ),
                "extension_leaf" => (
                    RelationType::ExtensionLeaf,
                    Rel {
                        rel_type: Some(RelType::ExtensionLeaf(
                            ::substrait::proto::ExtensionLeafRel::default(),
                        )),
                    },
                ),
                "extension_single" => (
                    RelationType::ExtensionSingle,
                    Rel {
                        rel_type: Some(RelType::ExtensionSingle(Box::default())),
                    },
                ),
                "extension_multi" => (
                    RelationType::ExtensionMulti,
                    Rel {
                        rel_type: Some(RelType::ExtensionMulti(
                            ::substrait::proto::ExtensionMultiRel::default(),
                        )),
                    },
                ),
                _ => (RelationType::Unknown, Rel::default()),
            }
        } else {
            (RelationType::Unknown, Rel::default())
        };

        // Create RelationData with the Rel protobuf
        let relation_data = RelationData::new(rel_protobuf);
        let blob = Some(Arc::new(Mutex::new(relation_data)) as Arc<Mutex<dyn Any + Send + Sync>>);

        // Define the relation in the symbol table with the blob
        let symbol = self.type_visitor.symbol_table_mut().define_symbol(
            name,
            location,
            SymbolType::Relation,
            Some(Box::new(relation_type)),
            blob,
        );

        eprintln!(
            "Created relation symbol '{}' with type {:?} and blob",
            symbol.name(),
            relation_type
        );

        // Set this as the current relation scope
        self.set_current_relation_scope(Some(symbol.clone()));

        Some(symbol)
    }

    /// Process a root relation and add it to the symbol table.
    fn process_root_relation(
        &mut self,
        ctx: &Root_relationContext<'input>,
    ) -> Option<Arc<SymbolInfo>> {
        // Create a location from the context's start token
        let token = ctx.start();
        let location = token_to_location(&token);

        // Root is a Relation symbol with RelationType::Root subtype (matching C++: kRelation + kRoot)
        // It indicates that the pipeline output should be wrapped in a RelRoot at the plan level
        // We need RelationData with an empty Rel for pipeline connectivity
        let mut relation_data = RelationData::new_empty();

        // Extract root output names from the grammar: ROOT { NAMES = [id, id, ...] }
        let root_names: Vec<String> = ctx
            .id_all()
            .iter()
            .map(|id_ctx| id_ctx.get_text())
            .collect();

        eprintln!(
            "Root relation has {} output names: {:?}",
            root_names.len(),
            root_names
        );

        // Store the root names in the RelationData
        relation_data.root_names = root_names;

        let blob = Some(Arc::new(Mutex::new(relation_data)) as Arc<Mutex<dyn Any + Send + Sync>>);

        // Define the root symbol as a Relation with Root subtype (matching C++)
        let symbol = self.type_visitor.symbol_table_mut().define_symbol(
            "root".to_string(),
            location,
            SymbolType::Relation,
            Some(Box::new(RelationType::Root)),
            blob,
        );

        eprintln!("Created root relation symbol 'root' with RelationType::Root and blob");

        // Set this as the current relation scope
        self.set_current_relation_scope(Some(symbol.clone()));

        Some(symbol)
    }

    /// Process a pipeline and add it to the symbol table.
    fn process_pipeline(&mut self, ctx: &PipelineContext<'input>) -> Option<Arc<SymbolInfo>> {
        // Pipeline symbols are created by PipelineVisitor via update_relation_symbol
        // This method is kept for potential future use but does not create symbols
        None
    }

    /// Process a schema definition and add it to the symbol table.
    fn process_schema(
        &mut self,
        ctx: &Schema_definitionContext<'input>,
    ) -> Option<Arc<SymbolInfo>> {
        // Extract schema name from the id token
        let schema_name = ctx.id()?.get_text();

        // Create a location from the context's start token
        // Get the start token directly - it's not an Option
        let token = ctx.start();
        let location = token_to_location(&token);

        // Define the schema in the symbol table
        // Symbol table accessed directly via base
        let symbol = self.type_visitor.symbol_table_mut().define_symbol(
            schema_name,
            location,
            SymbolType::Schema,
            None,
            None,
        );

        // Process schema items
        for schema_item in ctx.schema_item_all() {
            // We need to borrow each schema_item as a reference
            self.process_schema_item(&schema_item, &symbol);
        }

        Some(symbol)
    }

    /// Process a schema item and add it to the symbol table.
    fn process_schema_item(
        &mut self,
        ctx: &Schema_itemContext<'input>,
        parent_schema: &Arc<SymbolInfo>,
    ) -> Option<Arc<SymbolInfo>> {
        // Get the name of the column using schema item context trait
        let name = ctx.id(0)?.get_text();

        // Get the type from the literal_complex_type
        let type_text = if let Some(type_ctx) = ctx.literal_complex_type() {
            type_ctx.get_text()
        } else {
            "i64".to_string() // Default fallback
        };

        // Convert the type text to a Substrait Type protobuf
        let proto_type = self.type_visitor.text_to_type_proto(ctx, &type_text);

        // Store the Type protobuf in the blob
        let blob = Some(Arc::new(std::sync::Mutex::new(proto_type))
            as Arc<std::sync::Mutex<dyn std::any::Any + Send + Sync>>);

        // Create a location from the context's start token
        let token = ctx.start();
        let location = token_to_location(&token);

        // Define the column in the symbol table with the type blob
        let symbol = self.type_visitor.symbol_table_mut().define_symbol(
            name,
            location,
            SymbolType::SchemaColumn,
            None, // subtype
            blob, // blob contains the Type protobuf
        );

        // Set the schema as the parent
        symbol.set_schema(parent_schema.clone());
        Some(symbol)
    }
}

impl<'input> PlanVisitor<'input> for MainPlanVisitor<'input> {
    fn error_listener(&self) -> Arc<ErrorListener> {
        self.type_visitor.error_listener()
    }

    fn symbol_table(&self) -> SymbolTable {
        self.type_visitor.symbol_table()
    }
}

// ANTLR visitor implementation for MainPlanVisitor
impl<'input> ParseTreeVisitor<'input, SubstraitPlanParserContextType> for MainPlanVisitor<'input> {}

impl<'input> SubstraitPlanParserVisitor<'input> for MainPlanVisitor<'input> {
    // Override specific visitor methods for plan parsing

    fn visit_plan(&mut self, ctx: &PlanContext<'input>) {
        // Process the top-level plan structure
        println!("Visiting plan: {}", ctx.get_text());

        // Visit all children to process the entire plan
        self.visit_children(ctx);
    }

    fn visit_plan_detail(&mut self, ctx: &Plan_detailContext<'input>) {
        // Process plan details
        println!("Visiting plan detail: {}", ctx.get_text());

        // Visit children to process nested elements
        self.visit_children(ctx);
    }

    fn visit_pipelines(&mut self, ctx: &PipelinesContext<'input>) {
        // Process pipeline section
        println!("Visiting pipelines: {}", ctx.get_text());

        // Visit children to process individual pipelines
        self.visit_children(ctx);
    }

    fn visit_pipeline(&mut self, ctx: &PipelineContext<'input>) {
        // Process a single pipeline
        println!("Visiting pipeline: {}", ctx.get_text());

        // Process the pipeline and add it to the symbol table
        self.process_pipeline(ctx);

        // Visit children to process pipeline details
        self.visit_children(ctx);
    }

    fn visit_relation(&mut self, ctx: &RelationContext<'input>) {
        // Process a relation definition
        println!("Visiting relation: {}", ctx.get_text());

        // Process the relation and add it to the symbol table
        let relation_symbol = self.process_relation(ctx);

        // Visit children to process relation details (this sets schema, base_schema, etc.)
        self.visit_children(ctx);

        // NOTE: We DON'T call add_input_fields_to_schema here anymore.
        // Instead, it's called lazily from lookup_field_index when first needed.
        // This ensures all relations are fully processed (schemas linked, etc.) before
        // we try to populate field_references.

        // Clear the current relation scope when done with this relation
        self.set_current_relation_scope(None);
    }

    fn visit_root_relation(&mut self, ctx: &Root_relationContext<'input>) {
        // Process a root relation definition
        println!("Visiting root relation: {}", ctx.get_text());

        // Process the root relation and add it to the symbol table
        let relation_symbol = self.process_root_relation(ctx);

        // Visit children to process relation details (this sets schema, base_schema, etc.)
        self.visit_children(ctx);

        // NOTE: We DON'T call add_input_fields_to_schema here anymore.
        // Instead, it's called lazily from lookup_field_index when first needed.
        // This ensures all relations are fully processed (schemas linked, etc.) before
        // we try to populate field_references.

        // Clear the current relation scope when done with this relation
        self.set_current_relation_scope(None);
    }

    fn visit_schema_definition(&mut self, ctx: &Schema_definitionContext<'input>) {
        // Process a schema definition
        println!("Visiting schema definition: {}", ctx.get_text());

        // Process the schema and add it to the symbol table
        self.process_schema(ctx);

        // No need to visit children as process_schema already handles them
    }

    fn visit_extensionspace(&mut self, ctx: &ExtensionspaceContext<'input>) {
        // Process extension space definition
        println!("Visiting extension space: {}", ctx.get_text());
        self.num_spaces_seen += 1;

        // Process the extension space and add it to the symbol table
        let extension_space_symbol = self.process_extension_space(ctx);

        // Set as current extension space before visiting children (functions)
        let old_extension_space = self.current_extension_space.clone();
        self.current_extension_space = extension_space_symbol;

        // Visit children to process extension details (functions)
        self.visit_children(ctx);

        // Restore old extension space
        self.current_extension_space = old_extension_space;
    }

    fn visit_function(&mut self, ctx: &FunctionContext<'input>) {
        // Process function definition
        println!("Visiting function: {}", ctx.get_text());
        self.num_functions_seen += 1;

        // Process the function and add it to the symbol table
        self.process_function(ctx);

        // Visit children to process function details
        self.visit_children(ctx);
    }

    fn visit_source_definition(&mut self, ctx: &Source_definitionContext<'input>) {
        // Process source definition
        println!("Visiting source definition: {}", ctx.get_text());

        // Process the source and add it to the symbol table
        let source_symbol = self.process_source_definition(ctx);

        // Set current source scope before visiting children
        let old_scope = self.current_source_scope.clone();
        self.set_current_source_scope(source_symbol);

        // Visit children to process source details
        self.visit_children(ctx);

        // Restore previous scope
        self.set_current_source_scope(old_scope);
    }

    fn visit_named_table_detail(&mut self, ctx: &Named_table_detailContext<'input>) {
        // Process named table detail (strings in the source)
        println!("Visiting named table detail: {}", ctx.get_text());

        // Process the strings and add them to the symbol table
        // Clone the source symbol to avoid borrow issues
        if let Some(source_symbol) = self.current_source_scope().cloned() {
            self.process_named_table_detail(ctx, &source_symbol);
        }

        // Visit children to process any nested details
        self.visit_children(ctx);
    }

    // We delegate to the TypeVisitor for type-related nodes
    fn visit_literal_basic_type(&mut self, ctx: &Literal_basic_typeContext<'input>) {
        self.type_visitor.visit_literal_basic_type(ctx);
    }

    fn visit_literal_complex_type(&mut self, ctx: &Literal_complex_typeContext<'input>) {
        self.type_visitor.visit_literal_complex_type(ctx);
    }

    // We use the default implementation for other visitor methods,
    // which will call visit_children to traverse the tree
}
