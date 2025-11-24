// SPDX-License-Identifier: Apache-2.0

//! Relation visitor for processing relation definitions and expressions.

use std::rc::Rc;
use std::sync::Arc;

use antlr_rust::parser_rule_context::ParserRuleContext;
use antlr_rust::rule_context::RuleContext;
use antlr_rust::token::{GenericToken, Token};
use antlr_rust::tree::{ParseTree, ParseTreeVisitor};
use antlr_rust::TidExt;

use crate::textplan::common::structured_symbol_data::RelationData;
use crate::textplan::common::text_location::TextLocation;
use crate::textplan::parser::antlr::substraitplanparser::*;
use crate::textplan::parser::antlr::substraitplanparservisitor::SubstraitPlanParserVisitor;
use crate::textplan::parser::error_listener::ErrorListener;
use crate::textplan::symbol_table::{RelationType, SymbolInfo, SymbolTable, SymbolType};
use ::substrait::proto::rel::RelType;

use super::{token_to_location, PlanVisitor, TypeVisitor};

/// The RelationVisitor processes relation definitions and expressions.
///
/// This visitor is the fourth phase in the multiphase parsing approach.
pub struct RelationVisitor<'input> {
    symbol_table: SymbolTable,
    error_listener: Arc<ErrorListener>,
    current_relation_scope: Option<Arc<SymbolInfo>>,
    prescan_mode: bool,
    processing_emit: bool, // Track if we're currently processing an emit clause
    subquery_index_counters: std::collections::HashMap<String, i32>, // Track subquery indices per parent
    _phantom: std::marker::PhantomData<&'input ()>,
}

/// Helper function to parse sort direction from string
fn parse_sort_direction(text: &str) -> i32 {
    // Normalize the text: lowercase and remove underscores/special chars
    let normalized = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>();

    use ::substrait::proto::sort_field::SortDirection;
    match normalized.as_str() {
        "ascnullsfirst" => SortDirection::AscNullsFirst as i32,
        "ascnullslast" => SortDirection::AscNullsLast as i32,
        "descnullsfirst" => SortDirection::DescNullsFirst as i32,
        "descnullslast" => SortDirection::DescNullsLast as i32,
        "clustered" => SortDirection::Clustered as i32,
        _ => {
            eprintln!(
                "Unrecognized sort direction: {}, using ASC_NULLS_LAST",
                text
            );
            SortDirection::AscNullsLast as i32
        }
    }
}

impl<'input> RelationVisitor<'input> {
    /// Creates a new RelationVisitor.
    pub fn new(symbol_table: SymbolTable, error_listener: Arc<ErrorListener>) -> Self {
        Self {
            symbol_table,
            error_listener,
            current_relation_scope: None,
            prescan_mode: false,
            processing_emit: false,
            subquery_index_counters: std::collections::HashMap::new(),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Sets prescan mode - when true, only marks subqueries without building expressions.
    pub fn set_prescan_mode(&mut self, prescan: bool) {
        self.prescan_mode = prescan;
    }

    /// Gets and increments the next subquery index for a given parent relation.
    fn get_next_subquery_index(&mut self, parent_name: &str) -> i32 {
        let counter = self
            .subquery_index_counters
            .entry(parent_name.to_string())
            .or_insert(0);
        let index = *counter;
        *counter += 1;
        index
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

    /// Populates field_references for a relation from its input pipelines.
    /// This is called lazily when lookup_field_index needs field information.
    fn add_input_fields_to_schema(&mut self, relation_symbol: &Arc<SymbolInfo>) {
        use std::cell::RefCell;
        use std::collections::HashSet;

        thread_local! {
            static VISITING: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
        }

        // Check if we're currently visiting this relation (cycle detection)
        let is_visiting = VISITING.with(|v| v.borrow().contains(relation_symbol.name()));
        if is_visiting {
            println!(
                "    CYCLE DETECTED: Already visiting '{}', stopping recursion",
                relation_symbol.name()
            );
            return;
        }

        println!(
            "    add_input_fields_to_schema called for '{}'",
            relation_symbol.name()
        );

        // Check if already populated (early return to avoid unnecessary work)
        // But first ensure upstream generated fields are populated
        if let Some(blob_lock) = &relation_symbol.blob {
            if let Ok(blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                    if !relation_data.field_references.is_empty() {
                        println!(
                            "      '{}' already has {} field_references",
                            relation_symbol.name(),
                            relation_data.field_references.len()
                        );

                        // Before returning, ensure upstream generated fields are populated
                        // (This is needed because upstream may not have generated its fields yet)
                        let continuing = relation_data.continuing_pipeline.clone();
                        let new_pipes = relation_data.new_pipelines.clone();
                        drop(blob_data);

                        if let Some(cont) = continuing {
                            self.add_expressions_to_schema(&cont);
                        }
                        for pipe in &new_pipes {
                            self.add_expressions_to_schema(pipe);
                        }

                        return; // Already populated
                    }
                }
            }
        }

        // Mark as visiting
        VISITING.with(|v| v.borrow_mut().insert(relation_symbol.name().to_string()));

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
        // IMPORTANT: Don't hold locks while trying to acquire other locks to avoid deadlock!

        // First, collect upstream field references without holding our lock
        let mut collected_fields = Vec::new();

        // Check if this is a READ relation and needs schema fields
        let is_read_with_schema = if let Some(blob_lock) = &relation_symbol.blob {
            if let Ok(blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                    if let Some(RelType::Read(_)) = &relation_data.relation.rel_type {
                        relation_data.schema.clone()
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
        };

        if let Some(schema_arc) = is_read_with_schema {
            // For READ relations, collect fields from schema
            for symbol in self.symbol_table().symbols() {
                if symbol.symbol_type() == SymbolType::SchemaColumn {
                    if let Some(symbol_schema) = symbol.schema() {
                        if Arc::ptr_eq(&symbol_schema, &schema_arc) {
                            collected_fields.push(symbol.clone());
                        }
                    }
                }
            }
        } else {
            // For non-READ relations, collect fields from pipelines
            // Get continuing_pipeline and new_pipelines without holding lock
            let (continuing_pipeline, new_pipelines) =
                if let Some(blob_lock) = &relation_symbol.blob {
                    if let Ok(blob_data) = blob_lock.lock() {
                        if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                            (
                                relation_data.continuing_pipeline.clone(),
                                relation_data.new_pipelines.clone(),
                            )
                        } else {
                            (None, Vec::new())
                        }
                    } else {
                        (None, Vec::new())
                    }
                } else {
                    (None, Vec::new())
                };

            // Now process pipelines without holding our lock
            if let Some(continuing_pipeline) = continuing_pipeline {
                // Ensure the upstream relation's generated_field_references are populated first
                // This must be done before we copy its fields
                self.add_expressions_to_schema(&continuing_pipeline);

                if let Some(cont_blob_lock) = &continuing_pipeline.blob {
                    if let Ok(cont_blob_data) = cont_blob_lock.lock() {
                        if let Some(cont_relation_data) =
                            cont_blob_data.downcast_ref::<RelationData>()
                        {
                            if !cont_relation_data.output_field_references.is_empty() {
                                collected_fields
                                    .extend(cont_relation_data.output_field_references.clone());
                            } else {
                                collected_fields
                                    .extend(cont_relation_data.field_references.clone());
                                collected_fields
                                    .extend(cont_relation_data.generated_field_references.clone());
                            }
                        }
                    }
                }
            }

            for pipeline in &new_pipelines {
                // Ensure the upstream relation's generated_field_references are populated first
                self.add_expressions_to_schema(pipeline);

                if let Some(pipe_blob_lock) = &pipeline.blob {
                    if let Ok(pipe_blob_data) = pipe_blob_lock.lock() {
                        if let Some(pipe_relation_data) =
                            pipe_blob_data.downcast_ref::<RelationData>()
                        {
                            if !pipe_relation_data.output_field_references.is_empty() {
                                collected_fields
                                    .extend(pipe_relation_data.output_field_references.clone());
                            } else {
                                collected_fields
                                    .extend(pipe_relation_data.field_references.clone());
                                collected_fields
                                    .extend(pipe_relation_data.generated_field_references.clone());
                            }
                        }
                    }
                }
            }
        }

        // Finally, update our field_references with collected fields
        if let Some(blob_lock) = &relation_symbol.blob {
            if let Ok(mut blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_mut::<RelationData>() {
                    relation_data.field_references = collected_fields;
                }
            }
        }

        // Remove from visiting set
        VISITING.with(|v| v.borrow_mut().remove(relation_symbol.name()));

        println!(
            "    Finished populating field_references for '{}'",
            relation_symbol.name()
        );
    }

    /// Populates generated_field_references for a relation from its proto expressions.
    /// This is called after the relation details (expressions, measures, etc.) have been visited.
    /// Follows the C++ SubstraitPlanRelationVisitor::addExpressionsToSchema pattern.
    fn add_expressions_to_schema(&mut self, relation_symbol: &Arc<SymbolInfo>) {
        use substrait::proto::rel::RelType;

        println!(
            "    add_expressions_to_schema called for '{}'",
            relation_symbol.name()
        );

        // Phase 1: Collect expression information while holding the lock
        enum ExprInfo {
            FieldSelection(Arc<SymbolInfo>),
            ComplexExpression(Option<String>), // alias if exists
        }

        let expr_infos: Vec<ExprInfo> = if let Some(blob_lock) = &relation_symbol.blob {
            if let Ok(blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                    // Skip if already populated (idempotent)
                    if !relation_data.generated_field_references.is_empty() {
                        println!(
                            "      Already has {} generated_field_references, skipping",
                            relation_data.generated_field_references.len()
                        );
                        return;
                    }

                    // Process based on relation type
                    if let Some(rel_type) = &relation_data.relation.rel_type {
                        match rel_type {
                            RelType::Project(project_rel) => {
                                println!(
                                    "      Processing {} project expressions for '{}'",
                                    project_rel.expressions.len(),
                                    relation_symbol.name()
                                );

                                let mut infos = Vec::new();
                                for (expression_number, expr) in
                                    project_rel.expressions.iter().enumerate()
                                {
                                    // Check if this is a simple field selection
                                    if let Some(substrait::proto::expression::RexType::Selection(
                                        field_ref,
                                    )) = &expr.rex_type
                                    {
                                        if let Some(
                                            substrait::proto::expression::field_reference::ReferenceType::DirectReference(
                                                ref_segment,
                                            ),
                                        ) = &field_ref.reference_type
                                        {
                                            if let Some(
                                                substrait::proto::expression::reference_segment::ReferenceType::StructField(
                                                    struct_field,
                                                ),
                                            ) = &ref_segment.reference_type
                                            {
                                                // Simple field selection
                                                let field_index = struct_field.field as usize;
                                                if field_index < relation_data.field_references.len() {
                                                    let field_symbol = relation_data.field_references
                                                        [field_index]
                                                        .clone();
                                                    println!(
                                                        "        Expr {}: field selection -> will add '{}'",
                                                        expression_number, field_symbol.name()
                                                    );
                                                    infos.push(ExprInfo::FieldSelection(field_symbol));
                                                } else {
                                                    println!(
                                                        "        Expr {}: field index {} out of range",
                                                        expression_number, field_index
                                                    );
                                                }
                                                continue;
                                            }
                                        }
                                    }

                                    // Not a simple field selection - complex expression
                                    let alias = relation_data
                                        .generated_field_reference_aliases
                                        .get(&expression_number)
                                        .cloned();
                                    infos.push(ExprInfo::ComplexExpression(alias));
                                }

                                infos
                            }
                            _ => {
                                println!(
                                    "      Relation '{}' is not a project, skipping",
                                    relation_symbol.name()
                                );
                                Vec::new()
                            }
                        }
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

        // Phase 2: Create symbols without holding any locks
        // IMPORTANT: Both field selections AND complex expressions are added to generated_field_references.
        // Field selections reuse existing symbols but still occupy positions in the generated list.
        // This matches substrait-cpp behavior (SubstraitPlanRelationVisitor.cpp:1983).
        let mut generated_symbols = Vec::new();
        for (expr_num, info) in expr_infos.into_iter().enumerate() {
            match info {
                ExprInfo::FieldSelection(field_sym) => {
                    // Field selections reuse existing field symbols but add them to generated_field_references
                    // This is critical for emit to find the correct indices!
                    println!(
                        "        Expr {}: field selection '{}' -> adding to generated_field_references",
                        expr_num,
                        field_sym.name()
                    );
                    generated_symbols.push(field_sym);
                }
                ExprInfo::ComplexExpression(alias) => {
                    // Get unique name and create symbol
                    let unique_name = if let Some(alias_name) = alias {
                        alias_name
                    } else {
                        self.symbol_table_mut().get_unique_name("intermediate")
                    };

                    println!(
                        "        Expr {}: complex expression -> creating '{}'",
                        expr_num, unique_name
                    );

                    let new_symbol = self.symbol_table_mut().define_symbol(
                        unique_name,
                        relation_symbol.source_location().box_clone(),
                        SymbolType::Unknown,
                        None,
                        None,
                    );

                    generated_symbols.push(new_symbol);
                }
            }
        }

        // Phase 3: Add symbols to generated_field_references
        if !generated_symbols.is_empty() {
            if let Some(blob_lock) = &relation_symbol.blob {
                if let Ok(mut blob_data) = blob_lock.lock() {
                    if let Some(relation_data) = blob_data.downcast_mut::<RelationData>() {
                        relation_data.generated_field_references = generated_symbols;
                        println!(
                            "      Finished: {} generated_field_references for '{}'",
                            relation_data.generated_field_references.len(),
                            relation_symbol.name()
                        );
                    }
                }
            }
        }
    }

    /// Process a relation type and update the relation symbol.
    /// Note: This function is now deprecated - the relation type and blob
    /// are set in process_relation() when the symbol is created.
    fn process_relation_type(
        &mut self,
        _ctx: &Relation_typeContext<'input>,
        _relation_symbol: &Arc<SymbolInfo>,
    ) {
        // This function is no longer needed since we create the blob
        // when defining the symbol in process_relation()
    }

    /// Process a filter relation and add it to the symbol table.
    fn process_filter_relation(
        &mut self,
        ctx: &RelationFilterContext<'input>,
    ) -> Option<Arc<SymbolInfo>> {
        // Create a location from the context's start token
        let token = ctx.start();
        let location = token_to_location(&token);

        // Define the filter relation in the symbol table
        let symbol = self.symbol_table_mut().define_symbol(
            "filter".to_string(),
            location,
            SymbolType::Relation,
            Some(Box::new(RelationType::Filter)),
            None,
        );

        Some(symbol)
    }

    /// Process an expression relation and add it to the symbol table.
    fn process_expression_relation(
        &mut self,
        ctx: &RelationExpressionContext<'input>,
    ) -> Option<Arc<SymbolInfo>> {
        // Create a location from the context's start token
        // Get the start token directly - it's not an Option
        let token = ctx.start();
        let location = token_to_location(&token);

        // Define the expression relation in the symbol table
        let symbol = self.symbol_table_mut().define_symbol(
            "expression".to_string(),
            location,
            SymbolType::Relation,
            Some(Box::new(RelationType::Project)),
            None,
        );

        Some(symbol)
    }

    /// Process a join relation and add it to the symbol table.
    fn process_join_relation(
        &mut self,
        ctx: &RelationJoinTypeContext<'input>,
    ) -> Option<Arc<SymbolInfo>> {
        // Get the join type
        let join_type_text = ctx.get_text().to_lowercase();
        let join_type = if join_type_text.contains("hash") {
            RelationType::HashJoin
        } else if join_type_text.contains("merge") {
            RelationType::MergeJoin
        } else {
            RelationType::Join
        };

        // Create a location from the context's start token
        let token = ctx.start();
        let location = token_to_location(&token);

        // Define the join relation in the symbol table
        let symbol = self.symbol_table_mut().define_symbol(
            "join".to_string(),
            location,
            SymbolType::Relation,
            Some(Box::new(join_type)),
            None,
        );

        Some(symbol)
    }

    /// Process a constant expression and add it to the symbol table.
    fn process_constant_expression(
        &mut self,
        ctx: &ExpressionConstantContext<'input>,
    ) -> Option<Arc<SymbolInfo>> {
        // Get the constant value
        let value = ctx
            .constant()
            .map_or("unknown".to_string(), |c| c.get_text().to_string());

        // Create a location from the context's start token
        // Get the start token directly - it's not an Option
        let token = ctx.start();
        let location = token_to_location(&token);

        // Define the constant in the symbol table
        let symbol = self.symbol_table_mut().define_symbol(
            value.to_string(),
            location,
            SymbolType::Field,
            None,
            None,
        );

        Some(symbol)
    }

    /// Process a column reference expression and add it to the symbol table.
    fn process_column_expression(
        &mut self,
        ctx: &ExpressionColumnContext<'input>,
    ) -> Option<Arc<SymbolInfo>> {
        // Get the column name
        let name = ctx
            .column_name()
            .map_or("unnamed_column".to_string(), |c| c.get_text().to_string());

        // Create a location from the context's start token
        // Get the start token directly - it's not an Option
        let token = ctx.start();
        let location = token_to_location(&token);

        // Define the column in the symbol table
        let symbol = self.symbol_table_mut().define_symbol(
            name.to_string(),
            location,
            SymbolType::Field,
            None,
            None,
        );

        Some(symbol)
    }

    /// Process a function call expression and add it to the symbol table.
    fn process_function_expression(
        &mut self,
        ctx: &ExpressionFunctionUseContext<'input>,
    ) -> Option<Arc<SymbolInfo>> {
        // Get the function name - simplify for now
        let function_name = "unnamed_function"; // We'll fix this when we have the proper context

        // Create a location from the context's start token
        // Get the start token directly - it's not an Option
        let token = ctx.start();
        let location = token_to_location(&token);

        // Define the function call in the symbol table
        let symbol = self.symbol_table_mut().define_symbol(
            function_name.to_string(),
            location,
            SymbolType::Function,
            None,
            None,
        );

        Some(symbol)
    }

    /// Build an Expression protobuf from an expression AST node
    fn build_expression(
        &mut self,
        expr_ctx: &Rc<ExpressionContextAll<'input>>,
    ) -> ::substrait::proto::Expression {
        // In prescan mode, only process subquery expressions to mark them
        if self.prescan_mode {
            match expr_ctx.as_ref() {
                ExpressionContextAll::ExpressionScalarSubqueryContext(ctx) => {
                    return self.build_scalar_subquery(ctx);
                }
                ExpressionContextAll::ExpressionSetComparisonSubqueryContext(ctx) => {
                    return self.build_set_comparison_subquery(ctx);
                }
                ExpressionContextAll::ExpressionInPredicateSubqueryContext(ctx) => {
                    return self.build_in_predicate_subquery(ctx);
                }
                ExpressionContextAll::ExpressionFunctionUseContext(ctx) => {
                    // Still process function calls to find nested subqueries
                    return self.build_function_call(ctx);
                }
                _ => {
                    // Skip other expression types in prescan mode
                    return ::substrait::proto::Expression {
                        rex_type: Some(::substrait::proto::expression::RexType::Literal(
                            ::substrait::proto::expression::Literal {
                                literal_type: Some(
                                    ::substrait::proto::expression::literal::LiteralType::I64(0),
                                ),
                                nullable: false,
                                type_variation_reference: 0,
                            },
                        )),
                    };
                }
            }
        }

        // Match on the expression type
        match expr_ctx.as_ref() {
            ExpressionContextAll::ExpressionConstantContext(ctx) => {
                // Parse the constant value
                if let Some(constant_ctx) = ctx.constant() {
                    self.build_constant(&constant_ctx)
                } else {
                    // Fallback to placeholder
                    ::substrait::proto::Expression {
                        rex_type: Some(::substrait::proto::expression::RexType::Literal(
                            ::substrait::proto::expression::Literal {
                                literal_type: Some(
                                    ::substrait::proto::expression::literal::LiteralType::I64(0),
                                ),
                                nullable: false,
                                type_variation_reference: 0,
                            },
                        )),
                    }
                }
            }
            ExpressionContextAll::ExpressionColumnContext(ctx) => {
                println!("  Building column reference expression");
                self.build_column_reference(ctx)
            }
            ExpressionContextAll::ExpressionFunctionUseContext(ctx) => {
                println!("  Building function call expression");
                self.build_function_call(ctx)
            }
            ExpressionContextAll::ExpressionCastContext(ctx) => {
                println!("  Building cast expression");
                self.build_cast_expression(ctx)
            }
            ExpressionContextAll::ExpressionSetComparisonSubqueryContext(ctx) => {
                println!("  Building set comparison subquery expression");
                self.build_set_comparison_subquery(ctx)
            }
            ExpressionContextAll::ExpressionScalarSubqueryContext(ctx) => {
                println!("  Building scalar subquery expression");
                self.build_scalar_subquery(ctx)
            }
            ExpressionContextAll::ExpressionInPredicateSubqueryContext(ctx) => {
                println!("  Building IN predicate subquery expression");
                self.build_in_predicate_subquery(ctx)
            }
            ExpressionContextAll::ExpressionSetPredicateSubqueryContext(ctx) => {
                println!("  Building set predicate subquery expression");
                self.build_set_predicate_subquery(ctx)
            }
            _ => {
                println!("  Building unknown expression type (placeholder)");
                ::substrait::proto::Expression {
                    rex_type: Some(::substrait::proto::expression::RexType::Literal(
                        ::substrait::proto::expression::Literal {
                            literal_type: Some(
                                ::substrait::proto::expression::literal::LiteralType::I64(0),
                            ),
                            nullable: false,
                            type_variation_reference: 0,
                        },
                    )),
                }
            }
        }
    }

    /// Build a column reference expression from a column context
    fn build_column_reference(
        &mut self,
        ctx: &ExpressionColumnContext<'input>,
    ) -> ::substrait::proto::Expression {
        // Get the column name
        let column_name = ctx
            .column_name()
            .map(|c| c.get_text())
            .unwrap_or_else(|| "unknown".to_string());

        println!("    Column reference: {}", column_name);

        // Check if this is an outer reference (from a parent scope)
        let (field_index, steps_out) = self.lookup_field_with_scope(&column_name);

        println!(
            "      -> field index: {}, steps_out: {}",
            field_index, steps_out
        );

        // Create the appropriate root_type based on whether this is an outer reference
        let root_type = if steps_out > 0 {
            Some(
                ::substrait::proto::expression::field_reference::RootType::OuterReference(
                    ::substrait::proto::expression::field_reference::OuterReference {
                        steps_out: steps_out as u32,
                    },
                ),
            )
        } else {
            Some(
                ::substrait::proto::expression::field_reference::RootType::RootReference(
                    ::substrait::proto::expression::field_reference::RootReference {},
                ),
            )
        };

        ::substrait::proto::Expression {
            rex_type: Some(::substrait::proto::expression::RexType::Selection(Box::new(
                ::substrait::proto::expression::FieldReference {
                    reference_type: Some(::substrait::proto::expression::field_reference::ReferenceType::DirectReference(
                        ::substrait::proto::expression::ReferenceSegment {
                            reference_type: Some(::substrait::proto::expression::reference_segment::ReferenceType::StructField(Box::new(
                                ::substrait::proto::expression::reference_segment::StructField {
                                    field: field_index as i32,
                                    child: None,
                                }
                            ))),
                        }
                    )),
                    root_type,
                },
            ))),
        }
    }

    /// Get the parent query location for a relation, following C++ getParentQueryLocation.
    /// First checks the relation's own parent_query info, then checks pipeline_start.
    /// Returns the relation symbol that has parent_query info set, or None.
    fn find_relation_with_parent_query_info(
        &self,
        relation: &Arc<SymbolInfo>,
    ) -> Option<Arc<SymbolInfo>> {
        println!(
            "      DEBUG find_relation_with_parent_query_info: checking '{}', parent_index={}",
            relation.name(),
            relation.parent_query_index()
        );

        // First check if this relation has parent_query info
        if relation.parent_query_index() >= 0 {
            println!(
                "      DEBUG find_relation_with_parent_query_info: Found parent info on '{}'",
                relation.name()
            );
            return Some(relation.clone());
        }

        // If not, check pipeline_start
        if let Some(blob_lock) = &relation.blob {
            if let Ok(blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_ref::<crate::textplan::common::structured_symbol_data::RelationData>() {
                    if let Some(pipeline_start) = &relation_data.pipeline_start {
                        println!(
                            "      DEBUG find_relation_with_parent_query_info: checking pipeline_start '{}', parent_index={}",
                            pipeline_start.name(),
                            pipeline_start.parent_query_index()
                        );
                        if pipeline_start.parent_query_index() >= 0 {
                            println!(
                                "      DEBUG find_relation_with_parent_query_info: Found parent info on pipeline_start '{}'",
                                pipeline_start.name()
                            );
                            return Some(pipeline_start.clone());
                        }
                    }
                }
            }
        }

        None
    }

    /// Sets pipeline_start on all relations in a subquery pipeline.
    /// This follows the C++ PipelineVisitor pattern.
    fn set_pipeline_start_for_subquery(
        &self,
        subquery_root: &Arc<SymbolInfo>,
        subquery_index: i32,
    ) {
        println!(
            "      Setting pipeline_start for subquery rooted at '{}' with index {}",
            subquery_root.name(),
            subquery_index
        );

        // Walk the pipeline backward via continuing_pipeline and set both pipeline_start
        // AND parent_query_index on all relations in the subquery (following substrait-cpp)
        if let Some(blob_lock) = &subquery_root.blob {
            if let Ok(mut blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_mut::<crate::textplan::common::structured_symbol_data::RelationData>() {
                    // Set this relation's pipeline_start to itself
                    relation_data.pipeline_start = Some(subquery_root.clone());

                    // Walk the continuing_pipeline chain
                    let mut current = relation_data.continuing_pipeline.clone();
                    if let Some(ref cont) = current {
                    }
                    while let Some(current_rel) = current {
                        println!("        Setting pipeline_start and parent_query_index={} on '{}'", subquery_index, current_rel.name());

                        // Set parent_query_index to mark this relation as part of the subquery
                        // All members of the pipeline should have the same index
                        current_rel.set_parent_query_index(subquery_index);

                        if let Some(curr_blob_lock) = &current_rel.blob {
                            if let Ok(mut curr_blob_data) = curr_blob_lock.lock() {
                                if let Some(curr_relation_data) = curr_blob_data.downcast_mut::<crate::textplan::common::structured_symbol_data::RelationData>() {
                                    curr_relation_data.pipeline_start = Some(subquery_root.clone());
                                    current = curr_relation_data.continuing_pipeline.clone();
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Look up a field with scope information, returning (field_index, steps_out).
    /// If the field is found in the current relation, steps_out = 0.
    /// If the field is found in a parent relation, steps_out = number of levels up.
    fn lookup_field_with_scope(&mut self, column_name: &str) -> (usize, usize) {
        // First determine if this is an outer reference by checking if the schema belongs to a parent
        let steps_out = self.calculate_steps_out(column_name);

        // If it's an outer reference, look up the field index in the parent relation
        let field_index = if steps_out > 0 {
            println!(
                "      DEBUG: Outer reference detected, looking up '{}' in parent relation",
                column_name
            );
            self.lookup_field_index_in_parent_relation(column_name)
        } else {
            // Look up in current relation
            self.lookup_field_index(column_name)
        };

        (field_index, steps_out)
    }

    /// Look up a field index in the parent relation.
    /// This is used for outer references.
    fn lookup_field_index_in_parent_relation(&mut self, column_name: &str) -> usize {
        if let Some(current_rel) = self.current_relation_scope() {
            // Find the parent relation by checking parent_query_location
            let parent_loc = current_rel.parent_query_location();

            if let Some(parent) = self
                .symbol_table()
                .lookup_symbol_by_location_and_type(parent_loc.as_ref(), SymbolType::Relation)
            {
                println!(
                    "      DEBUG: Looking up '{}' in parent '{}'",
                    column_name,
                    parent.name()
                );

                // Ensure parent's field_references are populated
                if let Some(blob_lock) = &parent.blob {
                    if let Ok(blob_data) = blob_lock.lock() {
                        if let Some(relation_data) = blob_data.downcast_ref::<crate::textplan::common::structured_symbol_data::RelationData>() {
                            if relation_data.field_references.is_empty() {
                                drop(blob_data);
                                self.add_input_fields_to_schema(&parent);
                            }
                        }
                    }
                }

                // Now look up the field in the parent using the same logic as lookup_field_index
                // Parse column name - can be "field" or "schema.field"
                let (schema_name, field_name) = if let Some(dot_pos) = column_name.rfind('.') {
                    (&column_name[..dot_pos], &column_name[dot_pos + 1..])
                } else {
                    ("", column_name)
                };

                if let Some(blob_lock) = &parent.blob {
                    if let Ok(blob_data) = blob_lock.lock() {
                        if let Some(relation_data) = blob_data.downcast_ref::<crate::textplan::common::structured_symbol_data::RelationData>() {
                            // Search through field_references
                            for (index, field_sym) in relation_data.field_references.iter().enumerate() {
                                if self.field_matches(field_sym, field_name, schema_name, column_name) {
                                    return index;
                                }
                            }

                            // Also check generated_field_references
                            let field_ref_size = relation_data.field_references.len();
                            for (index, field_sym) in relation_data.generated_field_references.iter().enumerate() {
                                if self.field_matches(field_sym, field_name, schema_name, column_name) {
                                    let actual_index = field_ref_size + index;
                                    return actual_index;
                                }
                            }

                            println!("      WARNING: Field '{}' not found in parent '{}', defaulting to 0", column_name, parent.name());
                        }
                    }
                }
            } else {
                println!("      WARNING: Could not find parent relation for outer reference '{}', defaulting to 0", column_name);
            }
        }

        // Default to 0 if not found
        0
    }

    /// Calculate steps_out for a field reference to determine if it's an outer reference.
    /// Returns 0 if the field belongs to the current relation.
    /// Returns 1 if the field belongs to the parent relation (outer reference).
    fn calculate_steps_out(&self, column_name: &str) -> usize {
        // Parse column name to extract schema prefix
        let (schema_name, _field_name) = if let Some(dot_pos) = column_name.rfind('.') {
            (&column_name[..dot_pos], &column_name[dot_pos + 1..])
        } else {
            // No schema prefix - assume it's in current scope
            println!(
                "      DEBUG calculate_steps_out: no schema prefix for '{}', assuming local",
                column_name
            );
            return 0;
        };

        println!(
            "      DEBUG calculate_steps_out: checking '{}' with schema '{}'",
            column_name, schema_name
        );

        // If there's a schema prefix, check if it belongs to current or parent relation
        if let Some(current_rel) = self.current_relation_scope() {
            if let Some(schema_symbol) = self.symbol_table().lookup_symbol_by_name(schema_name) {
                println!(
                    "      DEBUG: found schema symbol '{}'",
                    schema_symbol.name()
                );

                // Check if schema belongs to current relation
                if let Some(blob_lock) = &current_rel.blob {
                    if let Ok(blob_data) = blob_lock.lock() {
                        if let Some(relation_data) = blob_data.downcast_ref::<crate::textplan::common::structured_symbol_data::RelationData>() {
                            if let Some(current_schema) = &relation_data.schema {
                                if Arc::ptr_eq(current_schema, &schema_symbol) {
                                    // Schema belongs to current relation - not an outer reference
                                    return 0;
                                }
                            }

                            // Schema doesn't match current relation - check if we're in a subquery
                            // Get parent query location
                            let parent_query_location = if current_rel.parent_query_index() >= 0 {
                                Some(current_rel.parent_query_location())
                            } else if let Some(pipeline_start) = &relation_data.pipeline_start {
                                if pipeline_start.parent_query_index() >= 0 {
                                    Some(pipeline_start.parent_query_location())
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            if let Some(parent_location) = parent_query_location {
                                // We're in a subquery - check if schema belongs to parent
                                if let Some(parent_rel) = self.symbol_table().lookup_symbol_by_location_and_type(
                                    parent_location.as_ref(),
                                    SymbolType::Relation,
                                ) {
                                    if let Some(parent_blob_lock) = &parent_rel.blob {
                                        if let Ok(parent_blob_data) = parent_blob_lock.lock() {
                                            if let Some(parent_relation_data) = parent_blob_data.downcast_ref::<crate::textplan::common::structured_symbol_data::RelationData>() {
                                                if let Some(parent_schema) = &parent_relation_data.schema {
                                                    if Arc::ptr_eq(parent_schema, &schema_symbol) {
                                                        // Schema belongs to parent - this is an outer reference!
                                                        println!("      ✓✓✓ '{}' IS AN OUTER REFERENCE (steps_out=1)", column_name);
                                                        return 1;
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
            }
        }

        // Not an outer reference
        0
    }

    /// Look up the field index from the current relation's field_references.
    /// This returns the index within the field_references vector, which is the
    /// position relative to this relation's input, not the absolute schema position.
    /// Falls back to schema-based lookup if field_references is not yet populated.
    /// Check if a field symbol matches the given name criteria
    /// Matches on: alias, name, or fully qualified name (schema.field)
    fn field_matches(
        &self,
        field_symbol: &Arc<SymbolInfo>,
        field_name: &str,
        schema_name: &str,
        full_column_name: &str,
    ) -> bool {
        // Check alias first (if present)
        if let Some(alias) = field_symbol.alias() {
            if alias == field_name || alias == full_column_name {
                return true;
            }
        }

        // Check field name
        if field_symbol.name() == field_name {
            // If schema prefix provided, verify it matches
            if !schema_name.is_empty() {
                if let Some(field_schema) = field_symbol.schema() {
                    return field_schema.name() == schema_name;
                }
                return false;
            }
            return true;
        }

        // Check fully qualified name (schema.field)
        if !schema_name.is_empty() {
            if let Some(field_schema) = field_symbol.schema() {
                let qualified = format!("{}.{}", field_schema.name(), field_symbol.name());
                if qualified == full_column_name {
                    return true;
                }
            }
        }

        false
    }

    fn lookup_field_index(&mut self, column_name: &str) -> usize {
        // Parse column name - can be "field" or "schema.field"
        let (schema_name, field_name) = if let Some(dot_pos) = column_name.rfind('.') {
            (&column_name[..dot_pos], &column_name[dot_pos + 1..])
        } else {
            // No schema prefix
            ("", column_name)
        };

        // Try to look up from current relation's field references
        // Search order (like C++): generated_field_references first, then field_references
        if let Some(relation_symbol) = self.current_relation_scope().cloned() {
            // Lazily populate field_references if empty (ensures all schemas are linked first)
            if let Some(blob_lock) = &relation_symbol.blob {
                if let Ok(blob_data) = blob_lock.lock() {
                    if let Some(relation_data) = blob_data.downcast_ref::<crate::textplan::common::structured_symbol_data::RelationData>() {
                        if relation_data.field_references.is_empty() && relation_data.generated_field_references.is_empty() {
                            drop(blob_data);
                            self.add_input_fields_to_schema(&relation_symbol);
                        }
                    }
                }
            }

            // Ensure generated field references from upstream are available
            // This must be done even if field_references is already populated
            if let Some(blob_lock) = &relation_symbol.blob {
                if let Ok(blob_data) = blob_lock.lock() {
                    if let Some(relation_data) = blob_data.downcast_ref::<crate::textplan::common::structured_symbol_data::RelationData>() {
                        // Get upstream relations to populate their generated fields
                        let continuing = relation_data.continuing_pipeline.clone();
                        let new_pipes = relation_data.new_pipelines.clone();
                        drop(blob_data);

                        // Ensure upstream relations have their generated fields populated
                        if let Some(cont) = continuing {
                            self.add_expressions_to_schema(&cont);
                        }
                        for pipe in &new_pipes {
                            self.add_expressions_to_schema(pipe);
                        }
                    }
                }
            }

            if let Some(blob_lock) = &relation_symbol.blob {
                if let Ok(blob_data) = blob_lock.lock() {
                    if let Some(relation_data) = blob_data.downcast_ref::<crate::textplan::common::structured_symbol_data::RelationData>() {
                        // Following C++ behavior: For aggregates during emit processing,
                        // set field_ref_size to 0 so lookups only find generated fields
                        // (C++ SubstraitPlanRelationVisitor::findFieldReferenceByName lines 2036-2038)
                        let is_aggregate = matches!(&relation_data.relation.rel_type, Some(::substrait::proto::rel::RelType::Aggregate(_)));
                        let field_ref_size = if is_aggregate && self.processing_emit {
                            0  // Aggregates during emit only expose generated fields
                        } else {
                            relation_data.field_references.len()
                        };

                        println!("      Looking up '{}' in relation '{}': field_refs={}, generated={}, processing_emit={}",
                            column_name, relation_symbol.name(), field_ref_size, relation_data.generated_field_references.len(), self.processing_emit);

                        // First search generated_field_references (in reverse order, like C++)
                        for (rev_idx, field_symbol) in relation_data.generated_field_references.iter().rev().enumerate() {
                            if self.field_matches(field_symbol, field_name, schema_name, column_name) {
                                let actual_idx = relation_data.generated_field_references.len() - rev_idx - 1;
                                return field_ref_size + actual_idx;
                            }
                        }

                        // Then search field_references (in reverse order, like C++)
                        for (rev_idx, field_symbol) in relation_data.field_references.iter().rev().enumerate() {
                            if self.field_matches(field_symbol, field_name, schema_name, column_name) {
                                return relation_data.field_references.len() - rev_idx - 1;
                            }
                        }

                        // Not found in either list
                        // Like substrait-cpp, when field_references are populated but field not found,
                        // just return a default value rather than doing global schema lookup
                        println!(
                            "      WARNING: Field '{}' not found in current relation (field_refs={}, generated={}), returning index 0",
                            column_name, field_ref_size, relation_data.generated_field_references.len()
                        );
                        return 0;
                    }
                }
            }
        }

        // If we reach here, field_references is not populated yet.
        // Only in this case, fall back to schema-based lookup (for initial parsing phase)
        if !schema_name.is_empty() {
            if let Some(schema_symbol) = self.symbol_table().lookup_symbol_by_name(schema_name) {
                if let Some(field_index) =
                    self.get_field_index_from_schema(&schema_symbol, field_name)
                {
                    return field_index;
                }
            }
        } else {
            // No schema prefix - try current relation's schema
            if let Some(relation_symbol) = self.current_relation_scope() {
                if let Some(blob_lock) = &relation_symbol.blob {
                    if let Ok(blob_data) = blob_lock.lock() {
                        if let Some(relation_data) = blob_data.downcast_ref::<crate::textplan::common::structured_symbol_data::RelationData>() {
                            if let Some(schema_arc) = &relation_data.schema {
                                if let Some(field_index) = self.get_field_index_from_schema(schema_arc, field_name) {
                                    return field_index;
                                }
                            }
                        }
                    }
                }
            }
        }

        println!(
            "      WARNING: Field '{}' not found anywhere, defaulting to index 0",
            column_name
        );
        // Default to 0 if not found
        0
    }

    /// Get field index from a schema symbol by looking up the field name
    fn get_field_index_from_schema(
        &self,
        schema_symbol: &Arc<SymbolInfo>,
        field_name: &str,
    ) -> Option<usize> {
        // Iterate through all symbols in the symbol table to find schema columns
        // that belong to this schema
        let mut index = 0;
        for symbol in self.symbol_table().symbols() {
            if symbol.symbol_type() == SymbolType::SchemaColumn {
                // Check if this column belongs to our schema
                if let Some(sym_schema) = symbol.schema() {
                    if Arc::ptr_eq(&sym_schema, schema_symbol) {
                        // This column belongs to our schema - check if name matches
                        if symbol.name() == field_name {
                            return Some(index);
                        }
                        index += 1;
                    }
                }
            }
        }
        None
    }

    /// Build an IfThen expression from IFTHEN function syntax
    fn build_if_then_expression(
        &mut self,
        ctx: &ExpressionFunctionUseContext<'input>,
    ) -> ::substrait::proto::Expression {
        // IFTHEN(if1, then1, [if2, then2, ...], else)
        // Arguments come in pairs (if, then), with the last odd argument being else
        let expr_ctxs = ctx.expression_all();
        let mut arguments: Vec<::substrait::proto::Expression> = Vec::new();

        for expr_ctx in expr_ctxs {
            arguments.push(self.build_expression(&expr_ctx));
        }

        let mut ifs = Vec::new();
        let mut else_expr = None;

        // Process arguments in pairs
        let mut i = 0;
        while i < arguments.len() {
            if i + 1 < arguments.len() {
                // We have a pair: if and then
                ifs.push(::substrait::proto::expression::if_then::IfClause {
                    r#if: Some(arguments[i].clone()),
                    then: Some(arguments[i + 1].clone()),
                });
                i += 2;
            } else {
                // Last odd argument is the else
                else_expr = Some(Box::new(arguments[i].clone()));
                i += 1;
            }
        }

        ::substrait::proto::Expression {
            rex_type: Some(::substrait::proto::expression::RexType::IfThen(Box::new(
                ::substrait::proto::expression::IfThen {
                    ifs,
                    r#else: else_expr,
                },
            ))),
        }
    }

    /// Build a function call expression from a function use context
    fn build_function_call(
        &mut self,
        ctx: &ExpressionFunctionUseContext<'input>,
    ) -> ::substrait::proto::Expression {
        // Get the function name
        let function_name = ctx
            .id()
            .map(|id| id.get_text())
            .unwrap_or_else(|| "unknown".to_string());

        println!("    Function call: {}", function_name);

        // Special case: IFTHEN is not a scalar function but an IfThen expression
        if function_name.eq_ignore_ascii_case("IFTHEN") {
            return self.build_if_then_expression(ctx);
        }

        // Look up function reference from symbol table
        let function_reference = self.lookup_function_reference(&function_name);
        println!("      -> function reference: {}", function_reference);

        // Recursively build arguments
        // Check if an expression is actually an enum argument (ends with _enum)
        let mut arguments = Vec::new();
        for expr_ctx in ctx.expression_all() {
            use crate::textplan::parser::antlr::substraitplanparser::ExpressionContextAll;

            // Check if this is a column reference ending in _enum
            if let ExpressionContextAll::ExpressionColumnContext(column_expr_ctx) =
                expr_ctx.as_ref()
            {
                if let Some(column_ctx) = column_expr_ctx.column_name() {
                    let column_text = column_ctx.get_text();
                    if column_text.ends_with("_enum") {
                        // This is an enum argument, not an expression
                        let enum_value = column_text.strip_suffix("_enum").unwrap().to_string();
                        println!("      Enum argument: {}", enum_value);
                        arguments.push(::substrait::proto::FunctionArgument {
                            arg_type: Some(::substrait::proto::function_argument::ArgType::Enum(
                                enum_value,
                            )),
                        });
                        continue;
                    }
                }
            }

            // Not an enum, build as expression
            let arg_expr = self.build_expression(&expr_ctx);
            arguments.push(::substrait::proto::FunctionArgument {
                arg_type: Some(::substrait::proto::function_argument::ArgType::Value(
                    arg_expr,
                )),
            });
        }

        println!("      with {} arguments", arguments.len());

        // Extract output type if present (from ARROW literal_complex_type)
        let output_type = if let Some(type_ctx) = ctx.literal_complex_type() {
            let type_text = type_ctx.get_text();
            // Create a temporary TypeVisitor to parse the type
            let type_visitor =
                TypeVisitor::new(self.symbol_table.clone(), self.error_listener.clone());
            Some(type_visitor.text_to_type_proto(ctx, &type_text))
        } else {
            None
        };

        ::substrait::proto::Expression {
            rex_type: Some(::substrait::proto::expression::RexType::ScalarFunction(
                ::substrait::proto::expression::ScalarFunction {
                    function_reference,
                    arguments,
                    output_type,
                    options: Vec::new(),
                    ..Default::default()
                },
            )),
        }
    }

    /// Build a cast expression from a cast context.
    fn build_cast_expression(
        &mut self,
        ctx: &ExpressionCastContext<'input>,
    ) -> ::substrait::proto::Expression {
        // Get the expression being cast
        let input_expr = if let Some(expr) = ctx.expression() {
            Box::new(self.build_expression(&expr))
        } else {
            // No input expression - return placeholder
            return ::substrait::proto::Expression {
                rex_type: Some(::substrait::proto::expression::RexType::Literal(
                    ::substrait::proto::expression::Literal {
                        literal_type: Some(
                            ::substrait::proto::expression::literal::LiteralType::I64(0),
                        ),
                        nullable: false,
                        type_variation_reference: 0,
                    },
                )),
            };
        };

        // Get the target type
        let mut target_type = if let Some(type_ctx) = ctx.literal_complex_type() {
            let type_text = type_ctx.get_text();
            // Create a temporary TypeVisitor to parse the type
            let type_visitor =
                TypeVisitor::new(self.symbol_table.clone(), self.error_listener.clone());
            type_visitor.text_to_type_proto(ctx, &type_text)
        } else {
            // No target type - return placeholder
            return ::substrait::proto::Expression {
                rex_type: Some(::substrait::proto::expression::RexType::Literal(
                    ::substrait::proto::expression::Literal {
                        literal_type: Some(
                            ::substrait::proto::expression::literal::LiteralType::I64(0),
                        ),
                        nullable: false,
                        type_variation_reference: 0,
                    },
                )),
            };
        };

        // Special handling: if casting a string literal to fixedchar/varchar without explicit length,
        // infer the length from the string
        if let Some(::substrait::proto::expression::RexType::Literal(literal)) =
            &input_expr.rex_type
        {
            if let Some(::substrait::proto::expression::literal::LiteralType::String(
                string_value,
            )) = &literal.literal_type
            {
                use ::substrait::proto::r#type::Kind;
                match &mut target_type.kind {
                    Some(Kind::FixedChar(ref mut fc_type)) if fc_type.length == 0 => {
                        // Infer length from string
                        fc_type.length = string_value.len() as i32;
                        println!("    Inferred fixedchar length: {}", fc_type.length);
                    }
                    Some(Kind::Varchar(ref mut vc_type)) if vc_type.length == 0 => {
                        // Infer length from string
                        vc_type.length = string_value.len() as i32;
                        println!("    Inferred varchar length: {}", vc_type.length);
                    }
                    _ => {}
                }
            }
        }

        ::substrait::proto::Expression {
            rex_type: Some(::substrait::proto::expression::RexType::Cast(Box::new(
                ::substrait::proto::expression::Cast {
                    r#type: Some(target_type),
                    input: Some(input_expr),
                    failure_behavior:
                        ::substrait::proto::expression::cast::FailureBehavior::Unspecified as i32,
                },
            ))),
        }
    }

    /// Extract a numeric value from a constant context.
    fn extract_number_from_constant(constant_ctx: &Rc<ConstantContextAll<'input>>) -> i32 {
        if let Some(number_token) = constant_ctx.NUMBER() {
            number_token.get_text().parse::<i32>().unwrap_or(0)
        } else {
            0
        }
    }

    /// Build a constant literal expression from a constant AST node
    fn build_constant(
        &self,
        constant_ctx: &Rc<ConstantContextAll<'input>>,
    ) -> ::substrait::proto::Expression {
        use ::substrait::proto::expression::literal::LiteralType;

        // Check what type of constant this is
        let literal_type = if let Some(number_token) = constant_ctx.NUMBER() {
            // Parse number literal
            let number_text = number_token.get_text();

            // Check if there's a type suffix (e.g., _date, _decimal<3,2>)
            if let Some(type_ctx) = constant_ctx.literal_basic_type() {
                // Get the type name
                let type_text = type_ctx.get_text();

                // Parse the type name
                let type_name = if let Some(id_ctx) = type_ctx.id() {
                    id_ctx.get_text().to_lowercase()
                } else {
                    "".to_string()
                };

                // Handle different literal types based on type name
                match type_name.as_str() {
                    "date" => {
                        if let Ok(days) = number_text.parse::<i32>() {
                            Some(LiteralType::Date(days))
                        } else {
                            Some(LiteralType::Date(0))
                        }
                    }
                    "time" => {
                        if let Ok(micros) = number_text.parse::<i64>() {
                            Some(LiteralType::Time(micros))
                        } else {
                            Some(LiteralType::Time(0))
                        }
                    }
                    "interval_year" => {
                        // Parse as years directly
                        if let Ok(years) = number_text.parse::<i32>() {
                            Some(LiteralType::IntervalYearToMonth(
                                ::substrait::proto::expression::literal::IntervalYearToMonth {
                                    years,
                                    months: 0,
                                },
                            ))
                        } else {
                            Some(LiteralType::IntervalYearToMonth(
                                ::substrait::proto::expression::literal::IntervalYearToMonth {
                                    years: 0,
                                    months: 0,
                                },
                            ))
                        }
                    }
                    "interval_year_month" => {
                        // Parse as total months, convert to years/months
                        if let Ok(total_months) = number_text.parse::<i32>() {
                            let years = total_months / 12;
                            let months = total_months % 12;
                            Some(LiteralType::IntervalYearToMonth(
                                ::substrait::proto::expression::literal::IntervalYearToMonth {
                                    years,
                                    months,
                                },
                            ))
                        } else {
                            Some(LiteralType::IntervalYearToMonth(
                                ::substrait::proto::expression::literal::IntervalYearToMonth {
                                    years: 0,
                                    months: 0,
                                },
                            ))
                        }
                    }
                    "decimal" => {
                        // Parse the numeric value and convert to i128 bytes
                        if let Ok(value) = number_text.parse::<i128>() {
                            let bytes = value.to_le_bytes().to_vec();

                            // Get precision and scale from literal_specifier if present
                            let (precision, scale) =
                                if let Some(spec_ctx) = type_ctx.literal_specifier() {
                                    // Parse numbers from the specifier (<precision,scale>)
                                    let numbers: Vec<i32> = spec_ctx
                                        .NUMBER_all()
                                        .iter()
                                        .filter_map(|n| n.get_text().parse::<i32>().ok())
                                        .collect();
                                    if numbers.len() >= 2 {
                                        (numbers[0], numbers[1])
                                    } else if numbers.len() == 1 {
                                        (numbers[0], 0)
                                    } else {
                                        (38, 0)
                                    }
                                } else {
                                    (38, 0) // Default precision and scale
                                };

                            Some(LiteralType::Decimal(
                                ::substrait::proto::expression::literal::Decimal {
                                    value: bytes,
                                    precision,
                                    scale,
                                },
                            ))
                        } else {
                            Some(LiteralType::Decimal(
                                ::substrait::proto::expression::literal::Decimal {
                                    value: vec![0; 16],
                                    precision: 38,
                                    scale: 0,
                                },
                            ))
                        }
                    }
                    "i8" => {
                        if let Ok(val) = number_text.parse::<i32>() {
                            Some(LiteralType::I8(val))
                        } else {
                            Some(LiteralType::I8(0))
                        }
                    }
                    "i16" => {
                        if let Ok(val) = number_text.parse::<i32>() {
                            Some(LiteralType::I16(val))
                        } else {
                            Some(LiteralType::I16(0))
                        }
                    }
                    "i32" => {
                        if let Ok(val) = number_text.parse::<i32>() {
                            Some(LiteralType::I32(val))
                        } else {
                            Some(LiteralType::I32(0))
                        }
                    }
                    "i64" => {
                        if let Ok(val) = number_text.parse::<i64>() {
                            Some(LiteralType::I64(val))
                        } else {
                            Some(LiteralType::I64(0))
                        }
                    }
                    "fp32" => {
                        if let Ok(val) = number_text.parse::<f32>() {
                            Some(LiteralType::Fp32(val))
                        } else {
                            Some(LiteralType::Fp32(0.0))
                        }
                    }
                    "fp64" => {
                        if let Ok(val) = number_text.parse::<f64>() {
                            Some(LiteralType::Fp64(val))
                        } else {
                            Some(LiteralType::Fp64(0.0))
                        }
                    }
                    _ => {
                        // Unknown type, default to i64
                        if let Ok(val) = number_text.parse::<i64>() {
                            Some(LiteralType::I64(val))
                        } else {
                            Some(LiteralType::I64(0))
                        }
                    }
                }
            } else {
                // No type suffix, try to parse as i32 first, then i64
                if let Ok(val) = number_text.parse::<i32>() {
                    Some(LiteralType::I32(val))
                } else if let Ok(val) = number_text.parse::<i64>() {
                    Some(LiteralType::I64(val))
                } else {
                    Some(LiteralType::I64(0))
                }
            }
        } else if let Some(string_token) = constant_ctx.STRING() {
            // Parse string literal (remove quotes)
            let string_text = string_token.get_text();
            let string_value = if string_text.starts_with('"') && string_text.ends_with('"') {
                string_text[1..string_text.len() - 1].to_string()
            } else {
                string_text.to_string()
            };

            // Check for type suffix for FixedChar or VarChar
            if let Some(type_ctx) = constant_ctx.literal_basic_type() {
                if let Some(id_ctx) = type_ctx.id() {
                    let type_name = id_ctx.get_text().to_lowercase();
                    match type_name.as_str() {
                        "fixedchar" => Some(LiteralType::FixedChar(string_value)),
                        "varchar" => Some(LiteralType::VarChar(
                            ::substrait::proto::expression::literal::VarChar {
                                value: string_value,
                                length: 0,
                            },
                        )),
                        _ => Some(LiteralType::String(string_value)),
                    }
                } else {
                    Some(LiteralType::String(string_value))
                }
            } else {
                Some(LiteralType::String(string_value))
            }
        } else if constant_ctx.TRUEVAL().is_some() {
            Some(LiteralType::Boolean(true))
        } else if constant_ctx.FALSEVAL().is_some() {
            Some(LiteralType::Boolean(false))
        } else if constant_ctx.NULLVAL().is_some() {
            Some(LiteralType::Null(::substrait::proto::Type::default()))
        } else if let Some(struct_ctx) = constant_ctx.struct_literal() {
            // Handle struct literals - check if it has an interval type suffix
            // NOTE: struct_literal uses literal_complex_type, not literal_basic_type (per grammar line 108)
            if let Some(type_ctx) = constant_ctx.literal_complex_type() {
                // Extract the type name from the complex type
                let type_text = type_ctx.get_text();
                let type_name = type_text.to_lowercase();

                // Parse the constants inside the struct
                let constants = struct_ctx.constant_all();

                match type_name.as_str() {
                    "interval_day_second" => {
                        // Expect {days, seconds, microseconds}_interval_day_second (deprecated format)
                        // IMPORTANT: Use PrecisionMode::Microseconds with microseconds field for compatibility
                        // The proto has both old (microseconds) and new (precision+subseconds) forms
                        if constants.len() >= 3 {
                            let days = Self::extract_number_from_constant(&constants[0]);
                            let seconds = Self::extract_number_from_constant(&constants[1]);
                            let microseconds = Self::extract_number_from_constant(&constants[2]);

                            use ::substrait::proto::expression::literal::interval_day_to_second::PrecisionMode;
                            Some(LiteralType::IntervalDayToSecond(
                                ::substrait::proto::expression::literal::IntervalDayToSecond {
                                    days,
                                    seconds,
                                    subseconds: 0, // Must be 0 when using microseconds
                                    precision_mode: Some(PrecisionMode::Microseconds(microseconds)),
                                },
                            ))
                        } else {
                            // Not enough components - create zero interval
                            use ::substrait::proto::expression::literal::interval_day_to_second::PrecisionMode;
                            Some(LiteralType::IntervalDayToSecond(
                                ::substrait::proto::expression::literal::IntervalDayToSecond {
                                    days: 0,
                                    seconds: 0,
                                    subseconds: 0,
                                    precision_mode: Some(PrecisionMode::Microseconds(0)),
                                },
                            ))
                        }
                    }
                    "interval_year_month" => {
                        // Expect {years, months}
                        if constants.len() >= 2 {
                            let years = Self::extract_number_from_constant(&constants[0]);
                            let months = Self::extract_number_from_constant(&constants[1]);

                            Some(LiteralType::IntervalYearToMonth(
                                ::substrait::proto::expression::literal::IntervalYearToMonth {
                                    years,
                                    months,
                                },
                            ))
                        } else {
                            // Not enough components
                            Some(LiteralType::IntervalYearToMonth(
                                ::substrait::proto::expression::literal::IntervalYearToMonth {
                                    years: 0,
                                    months: 0,
                                },
                            ))
                        }
                    }
                    _ => {
                        // Unknown struct literal type
                        Some(LiteralType::I64(0))
                    }
                }
            } else {
                // Struct without type suffix
                Some(LiteralType::I64(0))
            }
        } else {
            // TODO: Handle map_literal
            // For now, default to i64(0)
            Some(LiteralType::I64(0))
        };

        ::substrait::proto::Expression {
            rex_type: Some(::substrait::proto::expression::RexType::Literal(
                ::substrait::proto::expression::Literal {
                    literal_type,
                    nullable: false,
                    type_variation_reference: 0,
                },
            )),
        }
    }

    /// Look up the function reference (anchor) from the symbol table
    fn lookup_function_reference(&self, function_name: &str) -> u32 {
        // Iterate through all symbols to find functions with matching name
        for symbol in self.symbol_table().symbols() {
            if symbol.symbol_type() == SymbolType::Function {
                // Check if the symbol name (alias) matches what we're looking for
                if symbol.name() == function_name {
                    // Get the function data from the blob to get the anchor
                    if let Some(blob_lock) = &symbol.blob {
                        if let Ok(blob_data) = blob_lock.lock() {
                            if let Some(function_data) = blob_data.downcast_ref::<crate::textplan::common::structured_symbol_data::FunctionData>() {
                                return function_data.anchor;
                            }
                        }
                    }
                }
            }
        }

        // Function not found - use default reference 0
        0
    }

    /// Build a set comparison subquery expression (e.g., expression LT ANY SUBQUERY relation)
    fn build_set_comparison_subquery(
        &mut self,
        ctx: &ExpressionSetComparisonSubqueryContext<'input>,
    ) -> ::substrait::proto::Expression {
        use ::substrait::proto::expression::subquery::set_comparison::{ComparisonOp, ReductionOp};

        // Extract left expression
        let left_expr = ctx
            .expression()
            .map(|expr| Box::new(self.build_expression(&expr)));

        // Extract comparison operator
        let comp_op_text = ctx
            .COMPARISON()
            .map(|t| t.get_text().to_uppercase())
            .unwrap_or_default();
        let comparison_op = match comp_op_text.as_str() {
            "LT" | "<" => ComparisonOp::Lt as i32,
            "GT" | ">" => ComparisonOp::Gt as i32,
            "EQ" | "=" | "==" => ComparisonOp::Eq as i32,
            "NE" | "!=" | "<>" => ComparisonOp::Ne as i32,
            "LE" | "<=" => ComparisonOp::Le as i32,
            "GE" | ">=" => ComparisonOp::Ge as i32,
            _ => ComparisonOp::Unspecified as i32,
        };

        // Extract reduction operator (ANY or ALL)
        let reduction_op = if ctx.ANY().is_some() {
            ReductionOp::Any as i32
        } else if ctx.ALL().is_some() {
            ReductionOp::All as i32
        } else {
            ReductionOp::Unspecified as i32
        };

        // Extract subquery relation reference
        let relation_name = ctx
            .relation_ref()
            .map(|r| r.get_text())
            .unwrap_or_else(|| "unknown".to_string());

        println!(
            "    Set comparison subquery: left={:?}, comp_op={}, reduction_op={}, relation={}",
            left_expr.is_some(),
            comparison_op,
            reduction_op,
            relation_name
        );

        // Look up the subquery relation in the symbol table and mark it as a subquery
        if let Some(rel_symbol) = self.symbol_table().lookup_symbol_by_name(&relation_name) {
            // Mark this relation as a subquery by setting its parent query info
            // This will prevent it from being output as a top-level PlanRel in save_binary
            if let Some(parent_rel) = self.current_relation_scope() {
                let parent_name = parent_rel.name().to_string();
                let parent_location = parent_rel.source_location().box_clone();
                let subquery_index = self.get_next_subquery_index(&parent_name);
                println!(
                    "      Marking '{}' as subquery of '{}' with index {}",
                    relation_name, parent_name, subquery_index
                );
                rel_symbol.set_parent_query_location(parent_location);
                rel_symbol.set_parent_query_index(subquery_index);
            }
        } else {
            println!(
                "      WARNING: Subquery relation '{}' not found",
                relation_name
            );
        }

        // NOTE: Following Rust implementation philosophy, we do NOT copy the Rel protobuf here.
        // Instead, we leave `right` as None. Later, save_binary will build the Rel from the
        // symbol tree using add_inputs_to_relation, ensuring inputs come from pipeline connections.
        let right_rel = None;

        ::substrait::proto::Expression {
            rex_type: Some(::substrait::proto::expression::RexType::Subquery(Box::new(
                ::substrait::proto::expression::Subquery {
                    subquery_type: Some(
                        ::substrait::proto::expression::subquery::SubqueryType::SetComparison(
                            Box::new(::substrait::proto::expression::subquery::SetComparison {
                                left: left_expr,
                                comparison_op,
                                reduction_op,
                                right: right_rel,
                            }),
                        ),
                    ),
                },
            ))),
        }
    }

    /// Build a scalar subquery expression (e.g., SUBQUERY relation)
    fn build_scalar_subquery(
        &mut self,
        ctx: &ExpressionScalarSubqueryContext<'input>,
    ) -> ::substrait::proto::Expression {
        // Get the relation reference name (e.g., "SUBQUERY some_relation")
        if let Some(rel_ref) = ctx.relation_ref() {
            let relation_name = rel_ref.get_text();
            println!("    Scalar subquery: relation name = '{}'", relation_name);

            // Look up the relation symbol in the symbol table
            if let Some(relation_symbol) = self.symbol_table.lookup_symbol_by_name(&relation_name) {
                // Mark this relation as a subquery by setting its parent query info
                if let Some(parent_rel) = self.current_relation_scope() {
                    let parent_name = parent_rel.name().to_string();
                    let parent_loc = parent_rel.source_location().box_clone();
                    let parent_loc_hash = parent_rel.source_location().location_hash();
                    let subquery_index = self.get_next_subquery_index(&parent_name);
                    println!(
                        "      Marking '{}' as scalar subquery of '{}' with index {} (location hash: {})",
                        relation_name,
                        parent_name,
                        subquery_index,
                        parent_loc_hash
                    );
                    relation_symbol.set_parent_query_location(parent_loc);
                    relation_symbol.set_parent_query_index(subquery_index);

                    // Set pipeline_start on all relations in the subquery pipeline
                    // (following C++ PipelineVisitor pattern)
                    self.set_pipeline_start_for_subquery(&relation_symbol, subquery_index);
                }

                // Get the relation proto from the relation data
                if let Some(blob_lock) = &relation_symbol.blob {
                    if let Ok(blob_data) = blob_lock.lock() {
                        if let Some(relation_data) = blob_data.downcast_ref::<crate::textplan::common::structured_symbol_data::RelationData>() {
                            // Create the scalar subquery expression
                            // NOTE: We clone the relation proto here, which will later be rebuilt
                            // by save_binary to ensure nested subqueries are properly populated
                            return ::substrait::proto::Expression {
                                rex_type: Some(::substrait::proto::expression::RexType::Subquery(Box::new(
                                    ::substrait::proto::expression::Subquery {
                                        subquery_type: Some(::substrait::proto::expression::subquery::SubqueryType::Scalar(
                                            Box::new(::substrait::proto::expression::subquery::Scalar {
                                                input: Some(Box::new(relation_data.relation.clone())),
                                            }),
                                        )),
                                    },
                                ))),
                            };
                        }
                    }
                }
            } else {
                println!(
                    "    WARNING: Scalar subquery relation '{}' not found in symbol table",
                    relation_name
                );
            }
        }

        // Fallback: return a placeholder
        println!("    Scalar subquery: failed to build, returning placeholder");
        ::substrait::proto::Expression {
            rex_type: Some(::substrait::proto::expression::RexType::Literal(
                ::substrait::proto::expression::Literal {
                    literal_type: Some(::substrait::proto::expression::literal::LiteralType::I64(
                        0,
                    )),
                    nullable: false,
                    type_variation_reference: 0,
                },
            )),
        }
    }

    /// Build an IN predicate subquery expression (e.g., expression_list IN SUBQUERY relation)
    fn build_in_predicate_subquery(
        &mut self,
        ctx: &ExpressionInPredicateSubqueryContext<'input>,
    ) -> ::substrait::proto::Expression {
        // Extract needle expressions (left-hand side of IN)
        let mut needles = Vec::new();

        // Get expressions from expression_list
        if let Some(expr_list) = ctx.expression_list() {
            for expr in expr_list.expression_all() {
                needles.push(self.build_expression(&expr));
            }
        }

        // Extract subquery relation reference (haystack)
        let relation_name = ctx
            .relation_ref()
            .map(|r| r.get_text())
            .unwrap_or_else(|| "unknown".to_string());

        println!(
            "    IN predicate subquery: {} needles, haystack={}",
            needles.len(),
            relation_name
        );

        // Look up the subquery relation in the symbol table and mark it as a subquery
        if let Some(rel_symbol) = self.symbol_table().lookup_symbol_by_name(&relation_name) {
            // Mark this relation as a subquery by setting its parent query info
            if let Some(parent_rel) = self.current_relation_scope() {
                let parent_name = parent_rel.name().to_string();
                let parent_location = parent_rel.source_location().box_clone();
                let subquery_index = self.get_next_subquery_index(&parent_name);
                println!(
                    "      Marking '{}' as IN predicate subquery of '{}' with index {}",
                    relation_name, parent_name, subquery_index
                );
                rel_symbol.set_parent_query_location(parent_location);
                rel_symbol.set_parent_query_index(subquery_index);

                // Set pipeline_start on all relations in the subquery pipeline
                // (following C++ PipelineVisitor pattern and matching scalar subquery handling)
                self.set_pipeline_start_for_subquery(&rel_symbol, subquery_index);
            }
        } else {
            println!(
                "      WARNING: IN predicate subquery relation '{}' not found",
                relation_name
            );
        }

        // NOTE: Following the pattern from SetComparison, we do NOT copy the Rel protobuf here.
        // Instead, we leave `haystack` as None. Later, save_binary will build the Rel from the
        // symbol tree using add_inputs_to_relation, ensuring inputs come from pipeline connections.
        let haystack_rel = None;

        ::substrait::proto::Expression {
            rex_type: Some(::substrait::proto::expression::RexType::Subquery(Box::new(
                ::substrait::proto::expression::Subquery {
                    subquery_type: Some(
                        ::substrait::proto::expression::subquery::SubqueryType::InPredicate(
                            Box::new(::substrait::proto::expression::subquery::InPredicate {
                                needles,
                                haystack: haystack_rel,
                            }),
                        ),
                    ),
                },
            ))),
        }
    }

    /// Build a set predicate subquery expression (e.g., EXISTS IN SUBQUERY relation)
    fn build_set_predicate_subquery(
        &mut self,
        ctx: &ExpressionSetPredicateSubqueryContext<'input>,
    ) -> ::substrait::proto::Expression {
        // Determine predicate operator (EXISTS or UNIQUE)
        let predicate_op = if ctx.EXISTS().is_some() {
            ::substrait::proto::expression::subquery::set_predicate::PredicateOp::Exists as i32
        } else if ctx.UNIQUE().is_some() {
            ::substrait::proto::expression::subquery::set_predicate::PredicateOp::Unique as i32
        } else {
            ::substrait::proto::expression::subquery::set_predicate::PredicateOp::Unspecified as i32
        };

        // Extract subquery relation reference
        let relation_name = ctx
            .relation_ref()
            .map(|r| r.get_text())
            .unwrap_or_else(|| "unknown".to_string());

        println!(
            "    SET predicate subquery: op={}, relation={}",
            predicate_op, relation_name
        );

        // Look up the subquery relation in the symbol table and mark it as a subquery
        if let Some(rel_symbol) = self.symbol_table().lookup_symbol_by_name(&relation_name) {
            // Mark this relation as a subquery by setting its parent query info
            if let Some(parent_rel) = self.current_relation_scope() {
                let parent_name = parent_rel.name().to_string();
                let parent_location = parent_rel.source_location().box_clone();
                let subquery_index = self.get_next_subquery_index(&parent_name);
                println!(
                    "      Marking '{}' as SET predicate subquery of '{}' with index {}",
                    relation_name, parent_name, subquery_index
                );
                rel_symbol.set_parent_query_location(parent_location);
                rel_symbol.set_parent_query_index(subquery_index);

                // Set pipeline_start on all relations in the subquery pipeline
                // (following C++ PipelineVisitor pattern and matching scalar subquery handling)
                self.set_pipeline_start_for_subquery(&rel_symbol, subquery_index);
            }
        } else {
            println!(
                "      WARNING: SET predicate subquery relation '{}' not found",
                relation_name
            );
        }

        // NOTE: Following the pattern from InPredicate, we do NOT copy the Rel protobuf here.
        // Instead, we leave `tuples` as None. Later, save_binary will build the Rel from the
        // symbol tree using add_inputs_to_relation, ensuring inputs come from pipeline connections.
        let tuples_rel = None;

        ::substrait::proto::Expression {
            rex_type: Some(::substrait::proto::expression::RexType::Subquery(Box::new(
                ::substrait::proto::expression::Subquery {
                    subquery_type: Some(
                        ::substrait::proto::expression::subquery::SubqueryType::SetPredicate(
                            Box::new(::substrait::proto::expression::subquery::SetPredicate {
                                predicate_op,
                                tuples: tuples_rel,
                            }),
                        ),
                    ),
                },
            ))),
        }
    }
}

impl<'input> PlanVisitor<'input> for RelationVisitor<'input> {
    fn error_listener(&self) -> Arc<ErrorListener> {
        self.error_listener.clone()
    }

    fn symbol_table(&self) -> SymbolTable {
        self.symbol_table.clone()
    }
}

// ANTLR visitor implementation for RelationVisitor
impl<'input> ParseTreeVisitor<'input, SubstraitPlanParserContextType> for RelationVisitor<'input> {}

impl<'input> SubstraitPlanParserVisitor<'input> for RelationVisitor<'input> {
    // Override specific visitor methods for relation processing

    fn visit_relation(&mut self, ctx: &RelationContext<'input>) {
        // Look up the relation symbol that was already created in MainPlanVisitor phase
        // (C++ does: lookupSymbolByLocationAndType(Location(ctx), SymbolType::kRelation))
        let relation_ref = ctx.relation_ref();
        let symbol = if let Some(rel_ref) = relation_ref {
            if let Some(id) = rel_ref.id(0) {
                let name = id.get_text();
                self.symbol_table.lookup_symbol_by_name(&name)
            } else {
                None
            }
        } else {
            None
        };

        // Set as current scope if found
        if let Some(relation_symbol) = symbol.clone() {
            self.set_current_relation_scope(Some(relation_symbol.clone()));

            // IMPORTANT: Populate upstream generated fields BEFORE visiting children
            // This ensures that when expressions/measures are built, upstream generated
            // fields are already available for lookup
            if let Some(blob_lock) = &relation_symbol.blob {
                if let Ok(blob_data) = blob_lock.lock() {
                    if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                        let continuing = relation_data.continuing_pipeline.clone();
                        let new_pipes = relation_data.new_pipelines.clone();
                        drop(blob_data);

                        if let Some(cont) = continuing {
                            self.add_expressions_to_schema(&cont);
                        }
                        for pipe in &new_pipes {
                            self.add_expressions_to_schema(pipe);
                        }
                    }
                }
            }
        }

        // Continue visiting children to process relation details
        self.visit_children(ctx);

        // Add generated field references from expressions (after visiting relation details)
        // This matches the C++ addExpressionsToSchema pattern
        if let Some(relation_symbol) = symbol.clone() {
            self.add_expressions_to_schema(&relation_symbol);

            // Special handling for aggregates: copy generatedFieldReferences to outputFieldReferences
            // Following C++ SubstraitPlanRelationVisitor::visitRelation lines 387-393
            if let Some(blob_lock) = &relation_symbol.blob {
                if let Ok(blob_data) = blob_lock.lock() {
                    if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                        let is_aggregate = matches!(
                            &relation_data.relation.rel_type,
                            Some(::substrait::proto::rel::RelType::Aggregate(_))
                        );
                        if is_aggregate {
                            let generated_refs = relation_data.generated_field_references.clone();
                            drop(blob_data);

                            // Now update outputFieldReferences
                            if let Some(blob_lock) = &relation_symbol.blob {
                                if let Ok(mut blob_data) = blob_lock.lock() {
                                    if let Some(relation_data) =
                                        blob_data.downcast_mut::<RelationData>()
                                    {
                                        relation_data
                                            .output_field_references
                                            .extend(generated_refs);
                                        println!("  Aggregate '{}': copied {} generated fields to output_field_references",
                                            relation_symbol.name(),
                                            relation_data.output_field_references.len());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Clear scope when done
        self.set_current_relation_scope(None);
    }

    fn visit_root_relation(&mut self, ctx: &Root_relationContext<'input>) {
        // Look up the root symbol that was already created
        // Root relations are handled in MainPlanVisitor, just visit children here
        self.visit_children(ctx);
    }

    fn visit_relation_type(&mut self, ctx: &Relation_typeContext<'input>) {
        // This will be handled by visit_relation
        // Just visit children
        self.visit_children(ctx);
    }

    fn visit_relationFilter(&mut self, ctx: &RelationFilterContext<'input>) {
        // Add filter condition to the current relation (should be a Filter)
        // Grammar: relation_filter_behavior? FILTER expression SEMICOLON
        if let Some(relation_symbol) = self.current_relation_scope().cloned() {
            // Build the filter condition expression from AST
            let condition = if let Some(expr_ctx) = ctx.expression() {
                self.build_expression(&expr_ctx)
            } else {
                // No expression - use placeholder
                ::substrait::proto::Expression {
                    rex_type: Some(::substrait::proto::expression::RexType::Literal(
                        ::substrait::proto::expression::Literal {
                            literal_type: Some(
                                ::substrait::proto::expression::literal::LiteralType::Boolean(true),
                            ),
                            nullable: false,
                            type_variation_reference: 0,
                        },
                    )),
                }
            };

            // Add the condition to the FilterRel
            if let Some(blob_lock) = &relation_symbol.blob {
                if let Ok(mut blob_data) = blob_lock.lock() {
                    if let Some(relation_data) = blob_data.downcast_mut::<crate::textplan::common::structured_symbol_data::RelationData>() {
                        // Get mutable access to the Rel
                        if let Some(::substrait::proto::rel::RelType::Filter(ref mut filter_rel)) = relation_data.relation.rel_type {
                            filter_rel.condition = Some(Box::new(condition.clone()));
                            println!("  Added filter condition to filter relation '{}'", relation_symbol.name());

                            // Debug: count subquery expressions in condition
                            // Note: count_subqueries function was removed
                            // if let Some(cond_box) = &filter_rel.condition {
                            //     let count = count_subqueries(&cond_box);
                            //     println!("    Filter '{}' condition has {} subquery expressions", relation_symbol.name(), count);
                            // }
                        }
                    }
                }
            }
        }

        // Visit children to process any nested expressions
        self.visit_children(ctx);
    }

    fn visit_relationExpression(&mut self, ctx: &RelationExpressionContext<'input>) {
        // Add expression to the current relation (Project or Join)
        // Grammar: EXPRESSION expression SEMICOLON
        if let Some(relation_symbol) = self.current_relation_scope().cloned() {
            // Try to build actual expression from AST
            let expression = if let Some(expr_ctx) = ctx.expression() {
                self.build_expression(&expr_ctx)
            } else {
                // Fallback to placeholder if no expression context
                ::substrait::proto::Expression {
                    rex_type: Some(::substrait::proto::expression::RexType::Literal(
                        ::substrait::proto::expression::Literal {
                            literal_type: Some(
                                ::substrait::proto::expression::literal::LiteralType::I64(0),
                            ),
                            nullable: false,
                            type_variation_reference: 0,
                        },
                    )),
                }
            };

            // Add the expression to the appropriate relation type
            if let Some(blob_lock) = &relation_symbol.blob {
                if let Ok(mut blob_data) = blob_lock.lock() {
                    if let Some(relation_data) = blob_data.downcast_mut::<crate::textplan::common::structured_symbol_data::RelationData>() {
                        // Get mutable access to the Rel
                        use ::substrait::proto::rel::RelType;
                        match &mut relation_data.relation.rel_type {
                            Some(RelType::Project(ref mut project_rel)) => {
                                project_rel.expressions.push(expression);
                                println!("  Added expression to project relation '{}'", relation_symbol.name());
                            }
                            Some(RelType::Join(ref mut join_rel)) => {
                                join_rel.expression = Some(Box::new(expression));
                                println!("  Set join expression on join relation '{}'", relation_symbol.name());
                            }
                            Some(RelType::HashJoin(ref mut join_rel)) => {
                                // HashJoinRel uses post_join_filter, not expression
                                join_rel.post_join_filter = Some(Box::new(expression));
                                println!("  Set post-join filter on hash join relation '{}'", relation_symbol.name());
                            }
                            Some(RelType::MergeJoin(ref mut join_rel)) => {
                                // MergeJoinRel uses post_join_filter, not expression
                                join_rel.post_join_filter = Some(Box::new(expression));
                                println!("  Set post-join filter on merge join relation '{}'", relation_symbol.name());
                            }
                            _ => {
                                eprintln!("  Warning: EXPRESSION property used on unsupported relation type");
                            }
                        }
                    }
                }
            }
        }

        // Visit children to process any nested expressions
        self.visit_children(ctx);
    }

    #[allow(deprecated)]
    fn visit_relationGrouping(&mut self, ctx: &RelationGroupingContext<'input>) {
        // Add grouping expressions to the current relation (should be an Aggregate)
        // Grammar: GROUPING expression SEMICOLON
        // NOTE: Using deprecated Grouping.grouping_expressions format for now to match test data
        if let Some(relation_symbol) = self.current_relation_scope().cloned() {
            if let Some(expr_ctx) = ctx.expression() {
                // Build the grouping expression
                let expr = self.build_expression(&expr_ctx);

                // Add to deprecated Grouping.grouping_expressions (old format for roundtrip compatibility)
                if let Some(blob_lock) = &relation_symbol.blob {
                    if let Ok(mut blob_data) = blob_lock.lock() {
                        if let Some(relation_data) = blob_data.downcast_mut::<crate::textplan::common::structured_symbol_data::RelationData>() {
                            if let Some(::substrait::proto::rel::RelType::Aggregate(ref mut agg_rel)) = relation_data.relation.rel_type {
                                // Ensure there's at least one Grouping, or create one
                                if agg_rel.groupings.is_empty() {
                                    agg_rel.groupings.push(::substrait::proto::aggregate_rel::Grouping {
                                        grouping_expressions: Vec::new(),
                                        expression_references: Vec::new(),
                                    });
                                }

                                // Add expression directly to the deprecated field
                                agg_rel.groupings[0].grouping_expressions.push(expr.clone());

                                // Following C++ behavior: If this is a simple field selection,
                                // add the referenced field to generated_field_references
                                if let Some(::substrait::proto::expression::RexType::Selection(ref selection)) = expr.rex_type {
                                    // Check if this is a root reference (field selection from current relation)
                                    if let Some(::substrait::proto::expression::field_reference::RootType::RootReference(_)) = selection.root_type {
                                        // Check if it's a direct struct field reference
                                        if let Some(::substrait::proto::expression::field_reference::ReferenceType::DirectReference(ref ref_segment)) = selection.reference_type {
                                            if let Some(::substrait::proto::expression::reference_segment::ReferenceType::StructField(ref struct_field)) = ref_segment.reference_type {
                                                let field_index = struct_field.field as usize;
                                                if field_index < relation_data.field_references.len() {
                                                    let field_symbol = relation_data.field_references[field_index].clone();
                                                    relation_data.generated_field_references.push(field_symbol);
                                                    println!("  Added grouping field '{}' to generated_field_references",
                                                        relation_data.field_references[field_index].name());
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
        }

        // Visit children
        self.visit_children(ctx);
    }

    fn visit_relationSort(&mut self, ctx: &RelationSortContext<'input>) {
        // Add sort field to the current relation (should be a Sort)
        // Grammar: sort_field -> SORT expression (BY id)? SEMICOLON
        if let Some(relation_symbol) = self.current_relation_scope().cloned() {
            if let Some(sort_field_ctx) = ctx.sort_field() {
                if let Some(expr_ctx) = sort_field_ctx.expression() {
                    // Build the sort expression
                    let expr = self.build_expression(&expr_ctx);

                    // Parse the direction if provided
                    let direction = if let Some(id_ctx) = sort_field_ctx.id() {
                        parse_sort_direction(&id_ctx.get_text())
                    } else {
                        // Default to ASC NULLS LAST if not specified
                        ::substrait::proto::sort_field::SortDirection::AscNullsLast as i32
                    };

                    // Create the SortField
                    let sort_field = ::substrait::proto::SortField {
                        expr: Some(expr),
                        sort_kind: Some(::substrait::proto::sort_field::SortKind::Direction(
                            direction,
                        )),
                    };

                    // Add to SortRel
                    if let Some(blob_lock) = &relation_symbol.blob {
                        if let Ok(mut blob_data) = blob_lock.lock() {
                            if let Some(relation_data) = blob_data.downcast_mut::<crate::textplan::common::structured_symbol_data::RelationData>() {
                                if let Some(::substrait::proto::rel::RelType::Sort(ref mut sort_rel)) = relation_data.relation.rel_type {
                                    sort_rel.sorts.push(sort_field);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Visit children
        self.visit_children(ctx);
    }

    fn visit_relationMeasure(&mut self, ctx: &RelationMeasureContext<'input>) {
        // Add measures to the current relation (should be an Aggregate)
        // Grammar: MEASURE LEFTBRACE measure_detail* RIGHTBRACE
        if let Some(relation_symbol) = self.current_relation_scope().cloned() {
            // First pass: collect invocation if specified
            let mut invocation =
                ::substrait::proto::aggregate_function::AggregationInvocation::Unspecified;
            for measure_detail_ctx in ctx.measure_detail_all() {
                if measure_detail_ctx.INVOCATION().is_some() {
                    // Get the invocation value (id after INVOCATION keyword)
                    if let Some(id_ctx) = measure_detail_ctx.id(0) {
                        let invocation_str = id_ctx.get_text().to_lowercase();
                        invocation = match invocation_str.as_str() {
                            "all" => ::substrait::proto::aggregate_function::AggregationInvocation::All,
                            "distinct" => ::substrait::proto::aggregate_function::AggregationInvocation::Distinct,
                            _ => ::substrait::proto::aggregate_function::AggregationInvocation::Unspecified,
                        };
                    }
                }
            }

            // Second pass: process each measure_detail to build actual measures
            for measure_detail_ctx in ctx.measure_detail_all() {
                // Check if this is a MEASURE expression detail (not FILTER, INVOCATION, or sort)
                if let Some(expr_ctx) = measure_detail_ctx.expression() {
                    // For aggregate measures, we need to extract the function reference and arguments
                    // directly instead of building a ScalarFunction expression
                    let (function_reference, arguments, output_type) = match expr_ctx.as_ref() {
                        ExpressionContextAll::ExpressionFunctionUseContext(func_ctx) => {
                            // Get function name and look up reference
                            let function_name = func_ctx
                                .id()
                                .map(|id| id.get_text())
                                .unwrap_or_else(|| "unknown".to_string());
                            let func_ref = self.lookup_function_reference(&function_name);

                            // Build arguments directly
                            let mut args = Vec::new();
                            for arg_expr in func_ctx.expression_all() {
                                let expr = self.build_expression(&arg_expr);
                                args.push(::substrait::proto::FunctionArgument {
                                    arg_type: Some(
                                        ::substrait::proto::function_argument::ArgType::Value(expr),
                                    ),
                                });
                            }

                            // Extract output type if present
                            let out_type = if let Some(type_ctx) = func_ctx.literal_complex_type() {
                                let type_text = type_ctx.get_text();
                                let type_visitor = TypeVisitor::new(
                                    self.symbol_table.clone(),
                                    self.error_listener.clone(),
                                );
                                Some(type_visitor.text_to_type_proto(func_ctx, &type_text))
                            } else {
                                None
                            };

                            (func_ref, args, out_type)
                        }
                        _ => {
                            // For non-function expressions, wrap in arguments
                            let measure_expr = self.build_expression(&expr_ctx);
                            let arg = ::substrait::proto::FunctionArgument {
                                arg_type: Some(
                                    ::substrait::proto::function_argument::ArgType::Value(
                                        measure_expr,
                                    ),
                                ),
                            };
                            (0, vec![arg], None)
                        }
                    };

                    // Create the AggregateFunction
                    let agg_func = ::substrait::proto::AggregateFunction {
                        function_reference,
                        arguments,
                        output_type,
                        phase: ::substrait::proto::AggregationPhase::InitialToResult.into(),
                        invocation: invocation.into(),
                        ..Default::default()
                    };

                    // Create the Measure
                    let measure = ::substrait::proto::aggregate_rel::Measure {
                        measure: Some(agg_func),
                        filter: None,
                    };

                    // Extract NAMED alias if present (grammar: NAMED id)
                    // Check for NAMED keyword followed by an id
                    let measure_alias = if measure_detail_ctx.NAMED().is_some() {
                        // Get all id() contexts - the last one after NAMED is the alias
                        let ids = measure_detail_ctx.id_all();
                        // The grammar is: ... (ATSIGN id)? (NAMED id)?
                        // So if we have NAMED, the last id is the alias
                        if !ids.is_empty() {
                            Some(ids.last().unwrap().get_text())
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    // Add to the AggregateRel and track the measure index
                    let measure_index = if let Some(blob_lock) = &relation_symbol.blob {
                        if let Ok(mut blob_data) = blob_lock.lock() {
                            if let Some(relation_data) = blob_data.downcast_mut::<crate::textplan::common::structured_symbol_data::RelationData>() {
                                if let Some(::substrait::proto::rel::RelType::Aggregate(ref mut agg_rel)) = relation_data.relation.rel_type {
                                    let idx = agg_rel.measures.len();
                                    agg_rel.measures.push(measure);
                                    Some(idx)
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
                    };

                    // If there's a NAMED alias, add it to generated_field_references
                    if let (Some(alias), Some(measure_idx)) = (measure_alias, measure_index) {
                        if let Some(blob_lock) = &relation_symbol.blob {
                            if let Ok(mut blob_data) = blob_lock.lock() {
                                if let Some(relation_data) = blob_data.downcast_mut::<crate::textplan::common::structured_symbol_data::RelationData>() {
                                    // Calculate the field index for this measure
                                    // Aggregate outputs: grouping_fields + measures
                                    // Field index = number_of_grouping_fields + measure_index
                                    if let Some(::substrait::proto::rel::RelType::Aggregate(ref agg_rel)) = relation_data.relation.rel_type {
                                        let num_grouping_fields = agg_rel.groupings.first()
                                            .map(|g| g.grouping_expressions.len())
                                            .unwrap_or(0);
                                        let field_index = num_grouping_fields + measure_idx;

                                        // Create a symbol for this measure alias
                                        // Use get_unique_name to ensure uniqueness (matching converter behavior)
                                        // Use SymbolType::Measure to match converter behavior
                                        let location = TextLocation::new(0, 0);
                                        let unique_alias = self.symbol_table().get_unique_name(&alias);
                                        let measure_symbol = self.symbol_table_mut().define_symbol(
                                            unique_alias.clone(),
                                            location,
                                            SymbolType::Measure,
                                            None,
                                            None,
                                        );

                                        // Add to generated_field_references
                                        // Note: The field index is implicit based on position
                                        // For aggregates: grouping_fields come first, then measures
                                        relation_data.generated_field_references.push(measure_symbol);
                                        println!(
                                            "  Added measure alias '{}' (unique: '{}') as generated field at index {}",
                                            alias, unique_alias, field_index
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Visit children to process any nested expressions
        self.visit_children(ctx);
    }

    fn visit_relationJoinType(&mut self, ctx: &RelationJoinTypeContext<'input>) {
        // This handles the "TYPE <join_type>;" property inside a join relation definition.
        // Extract the join type (e.g., "LEFT", "INNER", "RIGHT", etc.)
        if let Some(id_node) = ctx.id() {
            let join_type_str = id_node.get_text().to_uppercase();

            // Map the join type string to the protobuf enum value
            let join_type_enum = match join_type_str.as_str() {
                "INNER" => 1,         // JOIN_TYPE_INNER
                "OUTER" => 2,         // JOIN_TYPE_OUTER
                "LEFT" => 3,          // JOIN_TYPE_LEFT
                "RIGHT" => 4,         // JOIN_TYPE_RIGHT
                "LEFT_SEMI" => 5,     // JOIN_TYPE_LEFT_SEMI
                "RIGHT_SEMI" => 6,    // JOIN_TYPE_RIGHT_SEMI
                "LEFT_ANTI" => 7,     // JOIN_TYPE_LEFT_ANTI
                "RIGHT_ANTI" => 8,    // JOIN_TYPE_RIGHT_ANTI
                "LEFT_SINGLE" => 9,   // JOIN_TYPE_LEFT_SINGLE
                "RIGHT_SINGLE" => 10, // JOIN_TYPE_RIGHT_SINGLE
                "LEFT_MARK" => 11,    // JOIN_TYPE_LEFT_MARK
                "RIGHT_MARK" => 12,   // JOIN_TYPE_RIGHT_MARK
                _ => 0,               // JOIN_TYPE_UNSPECIFIED
            };

            // Set the join type on the current relation (should be a join relation)
            if let Some(relation_symbol) = self.current_relation_scope() {
                if let Some(blob_lock) = &relation_symbol.blob {
                    if let Ok(mut blob_data) = blob_lock.lock() {
                        if let Some(relation_data) = blob_data.downcast_mut::<RelationData>() {
                            use ::substrait::proto::rel::RelType;
                            match &mut relation_data.relation.rel_type {
                                Some(RelType::Join(join_rel)) => {
                                    join_rel.r#type = join_type_enum;
                                    println!(
                                        "  Set join type to {} ({})",
                                        join_type_str, join_type_enum
                                    );
                                }
                                Some(RelType::HashJoin(join_rel)) => {
                                    join_rel.r#type = join_type_enum;
                                    println!(
                                        "  Set hash join type to {} ({})",
                                        join_type_str, join_type_enum
                                    );
                                }
                                Some(RelType::MergeJoin(join_rel)) => {
                                    join_rel.r#type = join_type_enum;
                                    println!(
                                        "  Set merge join type to {} ({})",
                                        join_type_str, join_type_enum
                                    );
                                }
                                _ => {
                                    eprintln!("  Warning: TYPE property used on non-join relation");
                                }
                            }
                        }
                    }
                }
            }
        }

        // Visit children
        self.visit_children(ctx);
    }

    fn visit_expressionConstant(&mut self, ctx: &ExpressionConstantContext<'input>) {
        // Process the constant expression
        self.process_constant_expression(ctx);

        // Visit children
        self.visit_children(ctx);
    }

    fn visit_expressionColumn(&mut self, ctx: &ExpressionColumnContext<'input>) {
        // Process the column reference expression
        self.process_column_expression(ctx);

        // Visit children
        self.visit_children(ctx);
    }

    fn visit_expressionFunctionUse(&mut self, ctx: &ExpressionFunctionUseContext<'input>) {
        // Process the function call expression
        self.process_function_expression(ctx);

        // Visit children
        self.visit_children(ctx);
    }

    fn visit_expressionCast(&mut self, ctx: &ExpressionCastContext<'input>) {
        // We'll just visit children for now
        self.visit_children(ctx);
    }

    fn visit_relationUsesSchema(&mut self, ctx: &RelationUsesSchemaContext<'input>) {
        // Link the current relation to its schema
        // Grammar: BASE_SCHEMA id SEMICOLON
        if let Some(relation_symbol) = self.current_relation_scope() {
            if let Some(schema_id) = ctx.id() {
                let schema_name = schema_id.get_text();
                if let Some(schema_symbol) = self.symbol_table.lookup_symbol_by_name(&schema_name) {
                    // Set the schema reference in the relation's RelationData
                    if let Some(blob_lock) = &relation_symbol.blob {
                        if let Ok(mut blob_data) = blob_lock.lock() {
                            if let Some(relation_data) = blob_data.downcast_mut::<crate::textplan::common::structured_symbol_data::RelationData>() {
                                relation_data.schema = Some(schema_symbol);
                                println!("  Linked relation '{}' to schema '{}'", relation_symbol.name(), schema_name);
                            }
                        }
                    }
                } else {
                    // Schema not found yet (might be defined later in textplan)
                    // Store the name for later resolution in save_binary
                    if let Some(blob_lock) = &relation_symbol.blob {
                        if let Ok(mut blob_data) = blob_lock.lock() {
                            if let Some(relation_data) = blob_data.downcast_mut::<crate::textplan::common::structured_symbol_data::RelationData>() {
                                relation_data.schema_name = Some(schema_name.to_string());
                            }
                        }
                    }
                }
            }
        }
        self.visit_children(ctx);
    }

    fn visit_relationSourceReference(&mut self, ctx: &RelationSourceReferenceContext<'input>) {
        // Link the current relation to its source
        // Grammar: source_reference SEMICOLON
        if let Some(relation_symbol) = self.current_relation_scope() {
            if let Some(source_ref_ctx) = ctx.source_reference() {
                if let Some(source_id) = source_ref_ctx.id() {
                    let source_name = source_id.get_text();
                    if let Some(source_symbol) =
                        self.symbol_table.lookup_symbol_by_name(&source_name)
                    {
                        // Set the source reference in the relation's RelationData
                        if let Some(blob_lock) = &relation_symbol.blob {
                            if let Ok(mut blob_data) = blob_lock.lock() {
                                if let Some(relation_data) = blob_data.downcast_mut::<crate::textplan::common::structured_symbol_data::RelationData>() {
                                    relation_data.source = Some(source_symbol);
                                    println!("  Linked relation '{}' to source '{}'", relation_symbol.name(), source_name);
                                }
                            }
                        }
                    }
                }
            }
        }
        self.visit_children(ctx);
    }

    fn visit_relationEmit(&mut self, ctx: &RelationEmitContext<'input>) {
        // CRITICAL: Populate generated_field_references BEFORE processing emit
        // Emit indices reference generated fields, so they must exist first
        if let Some(relation_symbol) = self.current_relation_scope().cloned() {
            self.add_expressions_to_schema(&relation_symbol);
        }

        // Mark that we're processing emit (affects field lookup behavior for aggregates)
        // Following C++ SubstraitPlanRelationVisitor which tracks processingEmit_
        self.processing_emit = true;

        // Handle EMIT column_name SEMICOLON
        // Emit specifies which fields from the input should be output
        if let Some(relation_symbol) = self.current_relation_scope().cloned() {
            if let Some(column_name_ctx) = ctx.column_name() {
                // Extract the column name (can be "id" or "id.id")
                let field_name = column_name_ctx.get_text();

                // Look up the field index in the current relation's field space
                let field_index = self.lookup_field_index(&field_name);

                println!(
                    "  Emit field '{}' at index {} in relation '{}'",
                    field_name,
                    field_index,
                    relation_symbol.name()
                );

                // Look up the actual field symbol to add to output_field_references
                if let Some(blob_lock) = &relation_symbol.blob {
                    if let Ok(blob_data) = blob_lock.lock() {
                        if let Some(relation_data) = blob_data.downcast_ref::<crate::textplan::common::structured_symbol_data::RelationData>() {
                            // Find the field symbol at this index
                            let field_symbol = if field_index < relation_data.field_references.len() {
                                Some(relation_data.field_references[field_index].clone())
                            } else {
                                let gen_index = field_index - relation_data.field_references.len();
                                if gen_index < relation_data.generated_field_references.len() {
                                    Some(relation_data.generated_field_references[gen_index].clone())
                                } else {
                                    None
                                }
                            };

                            drop(blob_data);

                            // Add to output_field_references
                            if let Some(field_sym) = field_symbol {
                                if let Some(blob_lock) = &relation_symbol.blob {
                                    if let Ok(mut blob_data) = blob_lock.lock() {
                                        if let Some(relation_data) = blob_data.downcast_mut::<crate::textplan::common::structured_symbol_data::RelationData>() {
                                            relation_data.output_field_references.push(field_sym);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        self.visit_children(ctx);

        // Reset processing_emit flag
        self.processing_emit = false;
    }

    fn visit_relationCount(&mut self, ctx: &RelationCountContext<'input>) {
        // Handle COUNT NUMBER SEMICOLON for Fetch relations
        if let Some(relation_symbol) = self.current_relation_scope().cloned() {
            if let Some(number_ctx) = ctx.NUMBER() {
                let count_text = number_ctx.get_text();
                if let Ok(count_value) = count_text.parse::<i64>() {
                    // Update the FetchRel with count
                    if let Some(blob_lock) = &relation_symbol.blob {
                        if let Ok(mut blob_data) = blob_lock.lock() {
                            if let Some(relation_data) = blob_data.downcast_mut::<crate::textplan::common::structured_symbol_data::RelationData>() {
                                use ::substrait::proto::rel::RelType;
                                if let Some(RelType::Fetch(ref mut fetch_rel)) = &mut relation_data.relation.rel_type {
                                    use ::substrait::proto::fetch_rel::CountMode;
                                    fetch_rel.count_mode = Some(CountMode::Count(count_value));
                                }
                            }
                        }
                    }
                }
            }
        }
        self.visit_children(ctx);
    }

    fn visit_relationOffset(&mut self, ctx: &RelationOffsetContext<'input>) {
        // Handle OFFSET NUMBER SEMICOLON for Fetch relations
        if let Some(relation_symbol) = self.current_relation_scope().cloned() {
            if let Some(number_ctx) = ctx.NUMBER() {
                let offset_text = number_ctx.get_text();
                if let Ok(offset_value) = offset_text.parse::<i64>() {
                    // Update the FetchRel with offset
                    if let Some(blob_lock) = &relation_symbol.blob {
                        if let Ok(mut blob_data) = blob_lock.lock() {
                            if let Some(relation_data) = blob_data.downcast_mut::<crate::textplan::common::structured_symbol_data::RelationData>() {
                                use ::substrait::proto::rel::RelType;
                                if let Some(RelType::Fetch(ref mut fetch_rel)) = &mut relation_data.relation.rel_type {
                                    use ::substrait::proto::fetch_rel::OffsetMode;
                                    fetch_rel.offset_mode = Some(OffsetMode::Offset(offset_value));
                                }
                            }
                        }
                    }
                }
            }
        }
        self.visit_children(ctx);
    }

    // We use the default implementation for other visitor methods,
    // which will call visit_children to traverse the tree
}
