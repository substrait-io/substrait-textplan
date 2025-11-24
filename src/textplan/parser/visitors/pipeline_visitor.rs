// SPDX-License-Identifier: Apache-2.0

//! Pipeline visitor for processing pipeline structures.

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

use super::{token_to_location, PlanVisitor};

/// The PipelineVisitor processes pipeline definitions.
///
/// This visitor is the third phase in the multiphase parsing approach.
pub struct PipelineVisitor<'input> {
    symbol_table: SymbolTable,
    error_listener: Arc<ErrorListener>,
    _phantom: std::marker::PhantomData<&'input ()>,
}

impl<'input> PipelineVisitor<'input> {
    /// Creates a new PipelineVisitor.
    pub fn new(symbol_table: SymbolTable, error_listener: Arc<ErrorListener>) -> Self {
        Self {
            symbol_table,
            error_listener,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Gets the symbol table.
    pub fn symbol_table(&self) -> SymbolTable {
        self.symbol_table.clone()
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

    /// Updates or creates a relation symbol for the given pipeline node.
    /// Matches C++ SubstraitPlanPipelineVisitor::updateRelationSymbol.
    ///
    /// If the symbol doesn't exist, creates a stub Relation symbol with
    /// RelationType::Unknown and empty RelationData to handle incomplete plans.
    fn update_relation_symbol(
        &mut self,
        ctx: &PipelineContext<'input>,
        relation_name: &str,
    ) -> Arc<SymbolInfo> {
        // Check if symbol already exists
        if let Some(symbol) = self.symbol_table.lookup_symbol_by_name(relation_name) {
            // Symbol exists, return it
            // Note: We don't call add_permanent_location here because it requires
            // exclusive access to the Arc, which we can't get if the symbol is
            // already referenced. The location was already set when the symbol
            // was first created.
            return symbol;
        }

        // Symbol doesn't exist - create a stub Relation with Unknown type
        println!(
            "  Creating stub Relation symbol '{}' (missing definition)",
            relation_name
        );
        let location = token_to_location(&ctx.start());
        let relation_data = RelationData::new_empty();
        let blob = Some(Arc::new(Mutex::new(relation_data)) as Arc<Mutex<dyn Any + Send + Sync>>);

        self.symbol_table.define_symbol(
            relation_name.to_string(),
            location,
            SymbolType::Relation,
            Some(Box::new(RelationType::Unknown)),
            blob,
        )
    }
}

impl<'input> PlanVisitor<'input> for PipelineVisitor<'input> {
    fn error_listener(&self) -> Arc<ErrorListener> {
        self.error_listener.clone()
    }

    fn symbol_table(&self) -> SymbolTable {
        self.symbol_table.clone()
    }
}

// ANTLR visitor implementation for PipelineVisitor
impl<'input> ParseTreeVisitor<'input, SubstraitPlanParserContextType> for PipelineVisitor<'input> {}

impl<'input> SubstraitPlanParserVisitor<'input> for PipelineVisitor<'input> {
    // Override specific visitor methods for pipeline processing

    fn visit_plan(&mut self, ctx: &PlanContext<'input>) {
        // Handle the plan node ourselves, not delegating to plan_visitor
        // This ensures our visit_pipeline override gets called
        println!("PipelineVisitor visiting plan node");
        self.visit_children(ctx);
        println!("PipelineVisitor finished visiting plan");
    }

    fn visit_pipelines(&mut self, ctx: &PipelinesContext<'input>) {
        // Process the pipelines section
        println!("PipelineVisitor processing pipelines: {}", ctx.get_text());

        // Only visit the direct pipeline children (top-level of each chain)
        // pipeline_all() returns ALL pipeline contexts including nested ones,
        // but we only want the direct children since visit_pipeline handles recursion
        let all_pipelines = ctx.pipeline_all();
        println!("  Found {} total pipeline contexts", all_pipelines.len());

        // Filter to only those whose parent is this pipelines context
        for pipeline in all_pipelines {
            if let Some(parent) = pipeline.get_parent_ctx() {
                // Check if the parent is a pipelines context (not another pipeline)
                if parent.get_rule_index() == RULE_pipelines {
                    println!("  Visiting top-level pipeline: {}", pipeline.get_text());
                    self.visit_pipeline(&pipeline);
                } else {
                    println!("  Skipping nested pipeline: {}", pipeline.get_text());
                }
            }
        }
    }

    fn visit_pipeline(&mut self, ctx: &PipelineContext<'input>) {
        // Following C++ SubstraitPlanPipelineVisitor::visitPipeline
        println!("PipelineVisitor processing pipeline: {}", ctx.get_text());

        // Get the relation name from this pipeline
        let relation_name = if let Some(relation_ref) = ctx.relation_ref() {
            println!("  Found relation_ref");
            if let Some(id) = relation_ref.id(0) {
                let name = id.get_text();
                println!("  Relation name: {}", name);
                name
            } else {
                println!("  No id(0) in relation_ref, returning early");
                return;
            }
        } else {
            println!("  No relation_ref, returning early");
            return;
        };

        // Ensure the symbol exists (create stub if missing)
        let symbol = self.update_relation_symbol(ctx, &relation_name);
        println!("  Using symbol '{}'", relation_name);

        // Process nested pipeline first (left-to-right processing)
        if let Some(nested_pipeline) = ctx.pipeline() {
            println!("  Processing nested pipeline for '{}'", relation_name);
            self.visit_pipeline(&nested_pipeline);
            println!("  Finished nested pipeline for '{}'", relation_name);
        }

        // Get the RelationData for this symbol
        let blob_lock = if let Some(blob) = &symbol.blob {
            blob
        } else {
            eprintln!("Warning: Symbol '{}' has no blob", relation_name);
            return;
        };

        let mut blob_data = match blob_lock.lock() {
            Ok(data) => data,
            Err(_) => {
                eprintln!("Warning: Failed to lock blob for '{}'", relation_name);
                return;
            }
        };

        let relation_data = if let Some(data) = blob_data.downcast_mut::<RelationData>() {
            data
        } else {
            eprintln!("Warning: Blob is not RelationData for '{}'", relation_name);
            return;
        };

        // Check for accidental cross-pipeline use
        if relation_data.continuing_pipeline.is_some() {
            eprintln!(
                "Error: Relation {} is already a non-terminating participant in a pipeline",
                relation_name
            );
            return;
        }

        // Get left symbol (nested pipeline)
        let left_symbol = if let Some(nested_pipeline) = ctx.pipeline() {
            if let Some(nested_ref) = nested_pipeline.relation_ref() {
                if let Some(nested_id) = nested_ref.id(0) {
                    let nested_name = nested_id.get_text();
                    self.symbol_table.lookup_symbol_by_name(&nested_name)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Get right symbol (parent pipeline)
        // In ANTLR, the parent context might be another pipeline or a root relation
        // Following C++ logic: if parent is a Pipeline context, look up symbol there
        let right_symbol: Option<Arc<SymbolInfo>> = {
            // Check if parent context exists and is a Pipeline
            if let Some(parent_ctx) = ctx.get_parent_ctx() {
                // Check if parent is a pipeline by checking rule index
                if parent_ctx.get_rule_index() == RULE_pipeline {
                    // Try to downcast parent to PipelineContext
                    if let Some(parent_pipeline) = parent_ctx.downcast_ref::<PipelineContext>() {
                        // Parent is a pipeline, get the relation name from it
                        if let Some(parent_ref) = parent_pipeline.relation_ref() {
                            if let Some(parent_id) = parent_ref.id(0) {
                                let parent_name = parent_id.get_text();
                                self.symbol_table.lookup_symbol_by_name(&parent_name)
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

        // Determine rightmost symbol (pipeline start)
        // For a terminus (no right_symbol), set pipeline_start to itself
        // For non-terminus, try to use left's pipeline_start
        let rightmost_symbol = if right_symbol.is_none() {
            // This is a terminus (no parent pipeline)
            println!(
                "  Pipeline: {} is a terminus, setting pipeline_start to itself",
                relation_name
            );
            Some(symbol.clone())
        } else if let Some(ref left) = left_symbol {
            // Has a parent, try to get pipeline_start from left
            if let Some(left_blob) = &left.blob {
                if let Ok(left_data) = left_blob.lock() {
                    if let Some(left_rel_data) = left_data.downcast_ref::<RelationData>() {
                        left_rel_data.pipeline_start.clone().or(Some(left.clone()))
                    } else {
                        Some(symbol.clone())
                    }
                } else {
                    Some(symbol.clone())
                }
            } else {
                Some(symbol.clone())
            }
        } else {
            // No left, we are standalone
            println!(
                "  Pipeline: {} is standalone, setting pipeline_start to itself",
                relation_name
            );
            Some(symbol.clone())
        };

        // Set pipeline start
        if let Some(rightmost) = rightmost_symbol {
            relation_data.pipeline_start = Some(rightmost);
        }

        // Connect to the left symbol
        if let Some(left) = left_symbol {
            // Determine the relation category for pipeline connections:
            // - Binary relations (Join, Cross, etc.) use new_pipelines for multiple inputs
            // - Root/terminal relations (Fetch, etc.) use new_pipelines to be identified as terminals
            // - Unary relations (Filter, Project, etc.) use continuing_pipeline for single input

            // Check if this is a terminal/root relation by name (since rel_type may not be set yet)
            // Only the "root" relation itself is terminal by name, not relations named "root1", "root2", etc.
            let is_root_by_name = relation_name == "root";

            let relation_category = if is_root_by_name {
                "terminal"
            } else {
                match &relation_data.relation.rel_type {
                    Some(::substrait::proto::rel::RelType::Join(_)) => "binary",
                    Some(::substrait::proto::rel::RelType::Cross(_)) => "binary",
                    Some(::substrait::proto::rel::RelType::Set(_)) => "binary",
                    Some(::substrait::proto::rel::RelType::HashJoin(_)) => "binary",
                    Some(::substrait::proto::rel::RelType::MergeJoin(_)) => "binary",
                    Some(::substrait::proto::rel::RelType::Fetch(_)) => "unary",
                    Some(::substrait::proto::rel::RelType::ExtensionSingle(_)) => "unary",
                    Some(::substrait::proto::rel::RelType::ExtensionLeaf(_)) => "terminal",
                    _ => "unary",
                }
            };

            if relation_category == "binary" || relation_category == "terminal" {
                // Binary or terminal relation: use new_pipelines for inputs
                println!(
                    "  Pipeline: {} ({} relation) adds new branch with left: {}",
                    relation_name,
                    relation_category,
                    left.name()
                );
                relation_data.new_pipelines.push(left);
            } else {
                // Unary relation: use continuing_pipeline for single input
                println!(
                    "  Pipeline: {} (unary relation) continues with left: {}",
                    relation_name,
                    left.name()
                );
                relation_data.continuing_pipeline = Some(left);
            }
        } else {
            println!("  Pipeline: {} has no left symbol", relation_name);
        }

        // Drop the lock before calling visit_children to avoid deadlock
        drop(blob_data);

        println!("  Finished setting up connections for '{}'", relation_name);

        // DON'T visit children - we already processed nested pipeline above
        // Calling visit_children here would cause infinite recursion
        // self.visit_children(ctx);

        println!("  Completed visit_pipeline for '{}'", relation_name);
    }

    // For plan_detail, continue traversing to find pipelines
    fn visit_plan_detail(&mut self, ctx: &Plan_detailContext<'input>) {
        // Continue traversing to find pipelines
        self.visit_children(ctx);
    }

    fn visit_relation(&mut self, ctx: &RelationContext<'input>) {
        // Skip - already processed
    }

    fn visit_extensionspace(&mut self, ctx: &ExtensionspaceContext<'input>) {
        // Skip - already processed
    }

    fn visit_function(&mut self, ctx: &FunctionContext<'input>) {
        // Skip - already processed
    }

    fn visit_source_definition(&mut self, ctx: &Source_definitionContext<'input>) {
        // Skip - already processed
    }

    fn visit_named_table_detail(&mut self, ctx: &Named_table_detailContext<'input>) {
        // Skip - already processed
    }

    // We use the default implementation for other visitor methods,
    // which will call visit_children to traverse the tree
}
