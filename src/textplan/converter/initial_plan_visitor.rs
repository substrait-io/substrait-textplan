// SPDX-License-Identifier: Apache-2.0

//! Plan visitor implementation for traversing Substrait plans.
//!
//! This module provides implementations of visitors for processing Substrait plans.
//! It builds on the generated BasePlanProtoVisitor trait to provide specialized
//! visitors for different stages of plan processing.

use crate::textplan::common::error::TextPlanError;
use crate::textplan::common::location::Location;
use crate::textplan::common::structured_symbol_data::ExtensionSpaceData;
use crate::textplan::common::structured_symbol_data::FunctionData;
use crate::textplan::common::structured_symbol_data::RelationData;
use crate::textplan::converter::generated::PlanProtoVisitor;
use crate::textplan::converter::generated::Traversable;
use crate::textplan::symbol_table::SourceType;
use crate::textplan::{ProtoLocation, SymbolInfo, SymbolType};
use ::substrait::proto as substrait;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

const ROOT_NAMES: &str = "root.names";

/// Initial visitor implementation that builds a symbol table from a Substrait plan.
///
/// This visitor is used to populate a symbol table with relationships between plan elements
/// during the first traversal of the plan.
pub struct InitialPlanVisitor {
    /// Symbol table for storing plan element information
    symbol_table: crate::textplan::symbol_table::SymbolTable,

    /// Current relation context for scope resolution
    current_relation_scope: Vec<Arc<String>>,

    /// Stack of ProtoLocations for current relations, used for parent_query tracking
    current_relation_locations: Vec<ProtoLocation>,

    internal_location: ProtoLocation,

    read_relation_sources: HashMap<String, Arc<SymbolInfo>>,
    read_relation_schemas: HashMap<String, Arc<SymbolInfo>>,

    /// Track the next subquery index for each parent relation (by location hash)
    subquery_indices: HashMap<u64, usize>,
}

fn short_name(s: &str) -> &str {
    match s.find(':') {
        Some(index) => &s[0..index],
        None => s,
    }
}

fn plan_rel_type_case_name(obj: &substrait::PlanRel) -> &'static str {
    if let Some(oneof) = &obj.rel_type {
        match oneof {
            substrait::plan_rel::RelType::Rel(_) => "rel",
            substrait::plan_rel::RelType::Root(_) => "root",
        }
    } else {
        "unknown"
    }
}

fn rel_type_case_name(relation: &substrait::Rel) -> &'static str {
    if let Some(oneof) = &relation.rel_type {
        match oneof {
            substrait::rel::RelType::Read(_) => "read",
            substrait::rel::RelType::Filter(_) => "filter",
            substrait::rel::RelType::Fetch(_) => "root",
            substrait::rel::RelType::Aggregate(_) => "aggregate",
            substrait::rel::RelType::Sort(_) => "sort",
            substrait::rel::RelType::Join(_) => "join",
            substrait::rel::RelType::Project(_) => "project",
            substrait::rel::RelType::Set(_) => "set",
            substrait::rel::RelType::ExtensionSingle(_) => "extension_single",
            substrait::rel::RelType::ExtensionMulti(_) => "extension_multi",
            substrait::rel::RelType::ExtensionLeaf(_) => "extension_leaf",
            substrait::rel::RelType::Cross(_) => "cross",
            substrait::rel::RelType::Reference(_) => "reference",
            substrait::rel::RelType::Write(_) => "write",
            substrait::rel::RelType::Ddl(_) => "ddl",
            substrait::rel::RelType::Update(_) => "update",
            substrait::rel::RelType::HashJoin(_) => "hash_join",
            substrait::rel::RelType::MergeJoin(_) => "merge_join",
            substrait::rel::RelType::NestedLoopJoin(_) => "nested_loop_join",
            substrait::rel::RelType::Window(_) => "window",
            substrait::rel::RelType::Exchange(_) => "exchange",
            substrait::rel::RelType::Expand(_) => "expand",
        }
    } else {
        "unknown"
    }
}

impl InitialPlanVisitor {
    /// Create a new initial visitor with the given symbol table
    pub fn new(symbol_table: crate::textplan::symbol_table::SymbolTable) -> Self {
        Self {
            symbol_table,
            current_relation_scope: Vec::new(),
            current_relation_locations: Vec::new(),
            internal_location: ProtoLocation::default(),
            read_relation_sources: HashMap::new(),
            read_relation_schemas: HashMap::new(),
            subquery_indices: HashMap::new(),
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

    /// Visit a relation root
    fn visit_relation_root(&mut self, root_rel: &substrait::RelRoot) -> Result<(), TextPlanError> {
        // Register the root relation in the symbol table
        for name in &root_rel.names {
            // Add the named relation to the symbol table
            self.symbol_table.add_root_relation(name);
        }

        Ok(())
    }

    /// Get the output mapping from a relation's common emit field.
    fn get_output_mapping(relation: &substrait::Rel) -> Vec<i32> {
        if let Some(rel_type) = &relation.rel_type {
            let common_opt = match rel_type {
                substrait::rel::RelType::Read(r) => r.common.as_ref(),
                substrait::rel::RelType::Filter(f) => f.common.as_ref(),
                substrait::rel::RelType::Fetch(f) => f.common.as_ref(),
                substrait::rel::RelType::Aggregate(a) => a.common.as_ref(),
                substrait::rel::RelType::Sort(s) => s.common.as_ref(),
                substrait::rel::RelType::Join(j) => j.common.as_ref(),
                substrait::rel::RelType::Project(p) => p.common.as_ref(),
                substrait::rel::RelType::Set(s) => s.common.as_ref(),
                substrait::rel::RelType::ExtensionSingle(e) => e.common.as_ref(),
                substrait::rel::RelType::ExtensionMulti(e) => e.common.as_ref(),
                substrait::rel::RelType::ExtensionLeaf(e) => e.common.as_ref(),
                substrait::rel::RelType::Cross(c) => c.common.as_ref(),
                substrait::rel::RelType::Reference(_) => None, // No common in ReferenceRel
                substrait::rel::RelType::Write(w) => w.common.as_ref(),
                substrait::rel::RelType::Ddl(d) => d.common.as_ref(),
                substrait::rel::RelType::HashJoin(h) => h.common.as_ref(),
                substrait::rel::RelType::MergeJoin(m) => m.common.as_ref(),
                substrait::rel::RelType::NestedLoopJoin(n) => n.common.as_ref(),
                substrait::rel::RelType::Window(w) => w.common.as_ref(),
                substrait::rel::RelType::Exchange(e) => e.common.as_ref(),
                substrait::rel::RelType::Expand(e) => e.common.as_ref(),
                substrait::rel::RelType::Update(_) => None, // UpdateRel has no common field
            };

            if let Some(common) = common_opt {
                if let Some(substrait::rel_common::EmitKind::Emit(emit)) = &common.emit_kind {
                    return emit.output_mapping.clone();
                }
            }
        }
        Vec::new()
    }

    /// Get the schema name from a field's schema reference.
    fn get_schema_name(field: &Arc<crate::textplan::SymbolInfo>) -> String {
        if let Some(schema) = field.schema() {
            schema.name().to_string()
        } else {
            String::new()
        }
    }

    /// Add a single field to a relation's field references.
    fn add_field_to_relation(
        relation_data: &mut RelationData,
        field: Arc<crate::textplan::SymbolInfo>,
    ) {
        relation_data.field_references.push(field);
    }

    /// Add fields from a single input relation to the current relation.
    fn add_fields_to_relation_single(
        &self,
        relation_data: &mut RelationData,
        relation: &substrait::Rel,
        relation_location: &ProtoLocation,
    ) {
        let symbol = self
            .symbol_table
            .lookup_symbol_by_location_and_type(relation_location, SymbolType::Relation);

        if let Some(symbol_ref) = symbol {
            if symbol_ref.symbol_type() != SymbolType::Relation {
                return;
            }

            symbol_ref.with_blob::<RelationData, _, _>(|input_relation_data| {
                if !input_relation_data.output_field_references.is_empty() {
                    for field in &input_relation_data.output_field_references {
                        Self::add_field_to_relation(relation_data, field.clone());
                    }
                } else {
                    for field in &input_relation_data.field_references {
                        Self::add_field_to_relation(relation_data, field.clone());
                    }
                    for field in &input_relation_data.generated_field_references {
                        Self::add_field_to_relation(relation_data, field.clone());
                    }
                }
            });
        }
    }

    /// Add fields from left and right input relations to the current relation.
    fn add_fields_to_relation_two(
        &self,
        relation_data: &mut RelationData,
        left: &substrait::Rel,
        left_location: &ProtoLocation,
        right: &substrait::Rel,
        right_location: &ProtoLocation,
    ) {
        self.add_fields_to_relation_single(relation_data, left, left_location);
        self.add_fields_to_relation_single(relation_data, right, right_location);
    }

    /// Add fields from multiple input relations (for Set, ExtensionMulti, etc.)
    fn add_fields_to_relation_multiple(
        &self,
        relation_data: &mut RelationData,
        relations: &[substrait::Rel],
        base_location: &ProtoLocation,
        field_name: &str,
    ) {
        for (i, relation) in relations.iter().enumerate() {
            let relation_location = base_location.indexed_field(field_name, i);
            self.add_fields_to_relation_single(relation_data, relation, &relation_location);
        }
    }

    /// Add grouping fields to a relation from an aggregate grouping.
    fn add_grouping_to_relation(
        &self,
        relation_data: &mut RelationData,
        grouping: &substrait::aggregate_rel::Grouping,
    ) {
        for expr in &grouping.grouping_expressions {
            // TODO -- Add support for groupings made up of complicated expressions.
            if let Some(substrait::expression::RexType::Selection(selection)) = &expr.rex_type {
                // TODO(REVIEW): Verify FieldReference.reference_type vs root_type usage.
                // The protobuf has both reference_type (DirectReference/MaskedReference) and
                // root_type (Expression/RootReference/OuterReference) as separate oneofs.
                if let Some(
                    substrait::expression::field_reference::ReferenceType::DirectReference(ref_seg),
                ) = &selection.reference_type
                {
                    if let Some(
                        substrait::expression::reference_segment::ReferenceType::StructField(
                            struct_field,
                        ),
                    ) = &ref_seg.reference_type
                    {
                        let mapping = struct_field.field as usize;
                        // TODO -- Figure out if we need to not add fields we've already seen.
                        if mapping >= relation_data.field_references.len() {
                            // errorListener_->addError("Grouping attempted to use a field reference not in the input field mapping.");
                            continue;
                        }
                        relation_data
                            .generated_field_references
                            .push(relation_data.field_references[mapping].clone());
                    }
                }
            }
        }
    }

    /// Update the local schema for a relation based on its type and inputs.
    /// This populates field_references, generated_field_references, and output_field_references.
    fn update_local_schema(
        &mut self,
        relation_data: &mut RelationData,
        relation: &substrait::Rel,
        internal_relation: &substrait::Rel,
    ) {
        if let Some(rel_type) = &relation.rel_type {
            match rel_type {
                substrait::rel::RelType::Read(read_rel) => {
                    if let Some(base_schema) = &read_rel.base_schema {
                        for (idx, name) in base_schema.names.iter().enumerate() {
                            // Get the corresponding type if available
                            let type_blob: Option<Arc<Mutex<dyn Any + Send + Sync>>> =
                                if let Some(struct_type) = &base_schema.r#struct {
                                    if idx < struct_type.types.len() {
                                        Some(Arc::new(Mutex::new(struct_type.types[idx].clone())))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                };

                            let symbol = self.symbol_table.define_symbol(
                                name.clone(),
                                self.current_location().field("read").field("base_schema"),
                                SymbolType::Field,
                                Some(Box::new(SourceType::Unknown)),
                                type_blob,
                            );

                            // Set the schema on the field symbol
                            if let Some(scope) = self.current_relation_scope.last() {
                                let scope_str = scope.as_ref().clone();
                                if let Some(schema_symbol) =
                                    self.read_relation_schemas.get(&scope_str)
                                {
                                    symbol.set_schema(schema_symbol.clone());
                                }
                            }

                            relation_data.field_references.push(symbol);
                        }
                    }
                }
                substrait::rel::RelType::Filter(filter_rel) => {
                    if let Some(input) = &filter_rel.input {
                        let input_location = self.current_location().field("filter").field("input");
                        self.add_fields_to_relation_single(relation_data, input, &input_location);
                    }
                }
                substrait::rel::RelType::Fetch(fetch_rel) => {
                    if let Some(input) = &fetch_rel.input {
                        let input_location = self.current_location().field("fetch").field("input");
                        self.add_fields_to_relation_single(relation_data, input, &input_location);
                    }
                }
                substrait::rel::RelType::Aggregate(agg_rel) => {
                    if let Some(input) = &agg_rel.input {
                        let input_location =
                            self.current_location().field("aggregate").field("input");
                        self.add_fields_to_relation_single(relation_data, input, &input_location);
                    }

                    for grouping in &agg_rel.groupings {
                        self.add_grouping_to_relation(relation_data, grouping);
                    }

                    // Add measures from internal_relation
                    if let Some(substrait::rel::RelType::Aggregate(internal_agg)) =
                        &internal_relation.rel_type
                    {
                        for measure in &internal_agg.measures {
                            let unique_name = self.symbol_table.get_unique_name("measurename");
                            let symbol = self.symbol_table.define_symbol(
                                unique_name,
                                self.current_location().field("aggregate").field("measures"),
                                SymbolType::Measure,
                                Some(Box::new(SourceType::Unknown)),
                                None,
                            );
                            relation_data.generated_field_references.push(symbol);
                        }
                    }

                    // TODO -- If there are multiple groupings add the additional output.
                    // Aggregate relations are different in that they alter the emitted fields by default.
                    relation_data
                        .output_field_references
                        .extend(relation_data.generated_field_references.iter().cloned());
                }
                substrait::rel::RelType::Sort(sort_rel) => {
                    if let Some(input) = &sort_rel.input {
                        let input_location = self.current_location().field("sort").field("input");
                        self.add_fields_to_relation_single(relation_data, input, &input_location);
                    }
                }
                substrait::rel::RelType::Join(join_rel) => {
                    if let (Some(left), Some(right)) = (&join_rel.left, &join_rel.right) {
                        let left_location = self.current_location().field("join").field("left");
                        let right_location = self.current_location().field("join").field("right");
                        self.add_fields_to_relation_two(
                            relation_data,
                            left,
                            &left_location,
                            right,
                            &right_location,
                        );
                    }
                }
                substrait::rel::RelType::Project(project_rel) => {
                    if let Some(input) = &project_rel.input {
                        let input_location =
                            self.current_location().field("project").field("input");
                        self.add_fields_to_relation_single(relation_data, input, &input_location);
                    }

                    for expr in &project_rel.expressions {
                        // TODO -- Add support for other kinds of direct references.
                        if let Some(substrait::expression::RexType::Selection(selection)) =
                            &expr.rex_type
                        {
                            // TODO(REVIEW): Verify FieldReference.reference_type vs root_type usage.
                            // The protobuf has both reference_type (DirectReference/MaskedReference) and
                            // root_type (Expression/RootReference/OuterReference) as separate oneofs.
                            if let Some(substrait::expression::field_reference::ReferenceType::DirectReference(ref_seg)) = &selection.reference_type {
                                if let Some(substrait::expression::reference_segment::ReferenceType::StructField(struct_field)) = &ref_seg.reference_type {
                                    let mapping = struct_field.field as usize;
                                    if mapping < relation_data.field_references.len() {
                                        let field = relation_data.field_references[mapping].clone();
                                        relation_data.generated_field_references.push(field.clone());

                                        // Handle duplicate field names needing schema qualification
                                        let prev_instance = relation_data.seen_field_reference_names.get(field.name());
                                        if field.alias().is_none() && prev_instance.is_some() {
                                            // Add a version with the schema supplied.
                                            let schema_name = Self::get_schema_name(&field);
                                            if !schema_name.is_empty() {
                                                relation_data.generated_field_reference_alternative_expression.insert(
                                                    relation_data.generated_field_references.len() - 1,
                                                    format!("{}.{}", schema_name, field.name())
                                                );
                                            }
                                            // Now update the first occurrence if it hasn't already.
                                            if let Some(&prev_idx) = prev_instance {
                                                let schema_name_prev = Self::get_schema_name(&relation_data.generated_field_references[prev_idx]);
                                                if !schema_name_prev.is_empty() {
                                                    relation_data.generated_field_reference_alternative_expression.insert(
                                                        prev_idx,
                                                        format!("{}.{}", schema_name_prev, field.name())
                                                    );
                                                }
                                            }
                                        }
                                        if field.alias().is_none() {
                                            relation_data.seen_field_reference_names.insert(
                                                field.name().to_string(),
                                                relation_data.generated_field_references.len() - 1
                                            );
                                        }
                                    } else {
                                        // TODO -- Add error handling
                                        // errorListener_->addError("Asked to project a field that isn't available");
                                    }
                                }
                            }
                        } else {
                            // Non-selection expression - create intermediate node
                            let unique_name = self.symbol_table.get_unique_name("intermediate");
                            let new_symbol = self.symbol_table.define_symbol(
                                unique_name.clone(),
                                self.current_location().field("project"),
                                SymbolType::Unknown,
                                None,
                                None,
                            );
                            relation_data
                                .generated_field_references
                                .push(new_symbol.clone());
                            self.symbol_table.add_alias(unique_name, &new_symbol);
                        }
                    }
                }
                substrait::rel::RelType::Set(set_rel) => {
                    let base_location = self.current_location().field("set");
                    self.add_fields_to_relation_multiple(
                        relation_data,
                        &set_rel.inputs,
                        &base_location,
                        "inputs",
                    );
                }
                substrait::rel::RelType::ExtensionSingle(ext_single) => {
                    if let Some(input) = &ext_single.input {
                        let input_location = self
                            .current_location()
                            .field("extension_single")
                            .field("input");
                        self.add_fields_to_relation_single(relation_data, input, &input_location);
                    }
                }
                substrait::rel::RelType::ExtensionMulti(ext_multi) => {
                    let base_location = self.current_location().field("extension_multi");
                    self.add_fields_to_relation_multiple(
                        relation_data,
                        &ext_multi.inputs,
                        &base_location,
                        "inputs",
                    );
                }
                substrait::rel::RelType::ExtensionLeaf(_) => {
                    // There is no defined way to get the schema for a leaf.
                }
                substrait::rel::RelType::Cross(cross_rel) => {
                    if let (Some(left), Some(right)) = (&cross_rel.left, &cross_rel.right) {
                        let left_location = self.current_location().field("cross").field("left");
                        let right_location = self.current_location().field("cross").field("right");
                        self.add_fields_to_relation_two(
                            relation_data,
                            left,
                            &left_location,
                            right,
                            &right_location,
                        );
                    }
                }
                substrait::rel::RelType::Reference(_) => {
                    // No schema for references
                }
                substrait::rel::RelType::Write(write_rel) => {
                    if let Some(input) = &write_rel.input {
                        let input_location = self.current_location().field("write").field("input");
                        self.add_fields_to_relation_single(relation_data, input, &input_location);
                    }
                }
                substrait::rel::RelType::Ddl(ddl_rel) => {
                    if let Some(view_def) = &ddl_rel.view_definition {
                        let input_location = self
                            .current_location()
                            .field("ddl")
                            .field("view_definition");
                        self.add_fields_to_relation_single(
                            relation_data,
                            view_def,
                            &input_location,
                        );
                    }
                }
                substrait::rel::RelType::HashJoin(hash_join) => {
                    if let (Some(left), Some(right)) = (&hash_join.left, &hash_join.right) {
                        let left_location =
                            self.current_location().field("hash_join").field("left");
                        let right_location =
                            self.current_location().field("hash_join").field("right");
                        self.add_fields_to_relation_two(
                            relation_data,
                            left,
                            &left_location,
                            right,
                            &right_location,
                        );
                    }
                }
                substrait::rel::RelType::MergeJoin(merge_join) => {
                    if let (Some(left), Some(right)) = (&merge_join.left, &merge_join.right) {
                        let left_location =
                            self.current_location().field("merge_join").field("left");
                        let right_location =
                            self.current_location().field("merge_join").field("right");
                        self.add_fields_to_relation_two(
                            relation_data,
                            left,
                            &left_location,
                            right,
                            &right_location,
                        );
                    }
                }
                substrait::rel::RelType::NestedLoopJoin(nested_join) => {
                    if let (Some(left), Some(right)) = (&nested_join.left, &nested_join.right) {
                        let left_location = self
                            .current_location()
                            .field("nested_loop_join")
                            .field("left");
                        let right_location = self
                            .current_location()
                            .field("nested_loop_join")
                            .field("right");
                        self.add_fields_to_relation_two(
                            relation_data,
                            left,
                            &left_location,
                            right,
                            &right_location,
                        );
                    }
                }
                substrait::rel::RelType::Window(window_rel) => {
                    if let Some(input) = &window_rel.input {
                        let input_location = self.current_location().field("window").field("input");
                        self.add_fields_to_relation_single(relation_data, input, &input_location);
                    }
                }
                substrait::rel::RelType::Exchange(exchange_rel) => {
                    if let Some(input) = &exchange_rel.input {
                        let input_location =
                            self.current_location().field("exchange").field("input");
                        self.add_fields_to_relation_single(relation_data, input, &input_location);
                    }
                }
                substrait::rel::RelType::Expand(expand_rel) => {
                    if let Some(input) = &expand_rel.input {
                        let input_location = self.current_location().field("expand").field("input");
                        self.add_fields_to_relation_single(relation_data, input, &input_location);
                    }
                }
                substrait::rel::RelType::Update(update_rel) => {
                    // TODO -- Add support for update relations
                }
            }
        }

        // Revamp the output based on the output mapping if present.
        let mapping = Self::get_output_mapping(relation);
        if !mapping.is_empty() {
            if matches!(
                relation.rel_type,
                Some(substrait::rel::RelType::Aggregate(_))
            ) {
                let generated_field_reference_size = relation_data.generated_field_references.len();
                relation_data.output_field_references.clear(); // Start over.
                for item in mapping {
                    let item_usize = item as usize;
                    if item_usize < generated_field_reference_size {
                        relation_data
                            .output_field_references
                            .push(relation_data.generated_field_references[item_usize].clone());
                    } else {
                        // TODO -- Add support for grouping fields (needs text syntax).
                        // errorListener_->addError("Asked to emit a field beyond what the aggregate produced.");
                    }
                }
                return;
            }
            for item in mapping {
                let item_usize = item as usize;
                let field_reference_size = relation_data.field_references.len();
                if item_usize < field_reference_size {
                    relation_data
                        .output_field_references
                        .push(relation_data.field_references[item_usize].clone());
                } else if item_usize
                    < field_reference_size + relation_data.generated_field_references.len()
                {
                    relation_data.output_field_references.push(
                        relation_data.generated_field_references[item_usize - field_reference_size]
                            .clone(),
                    );
                } else {
                    // errorListener_->addError("Asked to emit field which isn't available.");
                }
            }
        }
    }
}

impl PlanProtoVisitor for InitialPlanVisitor {
    // Workaround: The generated visitor doesn't traverse FunctionArgument.arg_type
    // So we need to do it manually here
    fn post_process_function_argument(&mut self, arg: &substrait::FunctionArgument) {
        use crate::textplan::converter::generated::base_plan_visitor::Traversable;

        if let Some(arg_type) = &arg.arg_type {
            if let substrait::function_argument::ArgType::Value(expr) = arg_type {
                // Manually traverse the expression
                let prev_location = self.current_location().clone();
                self.set_location(self.current_location().field("value"));
                expr.traverse(self);
                self.set_location(prev_location);
            }
        }
    }

    fn current_location(&self) -> &ProtoLocation {
        &self.internal_location
    }

    fn set_location(&mut self, location: ProtoLocation) {
        self.internal_location = location;
    }

    fn post_process_simple_extension_uri(
        &mut self,
        obj: &substrait::extensions::SimpleExtensionUri,
    ) {
        self.symbol_table.define_symbol(
            obj.uri.clone(),
            self.current_location().field("uri"),
            SymbolType::ExtensionSpace,
            /* subtype */ None,
            Some(Arc::new(Mutex::new(ExtensionSpaceData::new(
                obj.extension_uri_anchor,
            ))) as Arc<Mutex<dyn Any + Send + Sync>>),
        );
    }

    fn post_process_simple_extension_declaration(
        &mut self,
        obj: &substrait::extensions::SimpleExtensionDeclaration,
    ) {
        if let Some(mapping_type) = &obj.mapping_type {
            match mapping_type {
                substrait::extensions::simple_extension_declaration::MappingType::ExtensionFunction(ef) => {
                    let unique_name = self.symbol_table.get_unique_name(short_name(&ef.name));

                    self.symbol_table.define_symbol(unique_name,
                                                    self.current_location().field("extension_function"),
                                                    SymbolType::Function,
                                                    /* subtype */ None,
                                                    Some(Arc::new(Mutex::new(FunctionData::new(
                                                        ef.name.clone(),
                                                        Some(ef.extension_uri_reference),
                                                        ef.function_anchor),
                                                    )) as Arc<Mutex<dyn Any + Send + Sync>>)
                    );
                }
                _ => {
                    panic!("Unknown mapping type case {:#?} encountered.",
                           &obj.mapping_type);
                }
            }
        }
    }

    fn pre_process_plan_rel(&mut self, obj: &substrait::PlanRel) {
        let name = plan_rel_type_case_name(obj);
        let unique_name = self.symbol_table.get_unique_name(name);

        self.symbol_table.define_symbol(
            unique_name,
            self.current_location().clone(),
            SymbolType::PlanRelation,
            /* subtype */ None,
            Some(Arc::new(Mutex::new(RelationData::new(
                ::substrait::proto::Rel::default(),
            ))) as Arc<Mutex<dyn Any + Send + Sync>>),
        );
    }

    fn post_process_expression(&mut self, expression: &substrait::Expression) {
        if let Some(rex_type) = &expression.rex_type {
            // Check if it's a subquery
            if matches!(rex_type, substrait::expression::RexType::Subquery(_)) {
                println!(
                    "DEBUG INIT: ✓✓✓ Found SUBQUERY at {}",
                    self.current_location().path_string()
                );
            }

            if let substrait::expression::RexType::Subquery(subquery) = rex_type {
                println!(
                    "DEBUG INIT: Found subquery expression at location: {}",
                    self.current_location().path_string()
                );
                use substrait::expression::subquery::SubqueryType;

                // Determine the field path to the subquery relation
                let subquery_field_path = match &subquery.subquery_type {
                    Some(SubqueryType::Scalar(_)) => "subquery.scalar.input",
                    Some(SubqueryType::InPredicate(_)) => "subquery.in_predicate.haystack",
                    Some(SubqueryType::SetPredicate(_)) => "subquery.set_predicate.tuples",
                    Some(SubqueryType::SetComparison(_)) => "subquery.set_comparison.right",
                    None => return, // No subquery type, nothing to do
                };

                // Build the full location by extending current location with the path to the relation
                let mut rel_location = self.current_location().clone();
                for field_name in subquery_field_path.split('.') {
                    rel_location = rel_location.field(field_name);
                }

                // Look up the subquery relation symbol by its location
                println!(
                    "DEBUG INIT: Looking for subquery relation at location: {}",
                    rel_location.path_string()
                );

                if let Some(symbol) = self
                    .symbol_table
                    .lookup_symbol_by_location_and_type(&rel_location, SymbolType::Relation)
                {
                    // Set the parent query location to the current relation's location
                    // We stored the actual ProtoLocation in current_relation_locations
                    if let Some(parent_rel_location) = self.current_relation_locations.last() {
                        let parent_hash = parent_rel_location.location_hash();
                        println!(
                            "DEBUG INIT: Setting parent_query_location for '{}' to hash {}",
                            symbol.name(),
                            parent_hash
                        );
                        self.symbol_table
                            .set_parent_query_location(&symbol, parent_rel_location.box_clone());

                        // Get the next subquery index for this parent relation
                        let current_index = *self.subquery_indices.entry(parent_hash).or_insert(0);
                        println!(
                            "DEBUG INIT: Setting parent_query_index for '{}' to {}",
                            symbol.name(),
                            current_index
                        );
                        self.symbol_table
                            .set_parent_query_index(&symbol, current_index as i32);

                        // Increment for next subquery in same parent
                        *self.subquery_indices.get_mut(&parent_hash).unwrap() += 1;
                    }
                } else {
                    println!(
                        "DEBUG INIT: FAILED to find subquery relation at location: {}",
                        rel_location.path_string()
                    );
                }
            }
        }
    }

    fn pre_process_rel(&mut self, obj: &substrait::Rel) {
        let location_path = self.current_location().path_string();
        self.current_relation_scope.push(Arc::new(location_path));
        self.current_relation_locations
            .push(self.current_location().clone());
    }

    fn post_process_rel(&mut self, obj: &substrait::Rel) {
        let name = rel_type_case_name(obj);

        // Create relation data to store with the symbol.
        let mut relation_data = RelationData::new(obj.clone());

        // Clone the internal relation before the mutable borrow
        let internal_relation = relation_data.relation.clone();

        // Update the relation data for long term use.
        self.update_local_schema(&mut relation_data, obj, &internal_relation);

        if let Some(scope) = self.current_relation_scope.last() {
            let scope_str = scope.as_ref().clone();
            if let Some(source) = self.read_relation_sources.get(&scope_str) {
                relation_data.source = Some(source.clone());
            }
            if let Some(schema) = self.read_relation_schemas.get(&scope_str) {
                relation_data.schema = Some(schema.clone());
            }
        }

        // Finally create our entry in the symbol table.
        let unique_name = self.symbol_table.get_unique_name(name);
        self.symbol_table.define_symbol(
            unique_name,
            self.current_location().clone(),
            SymbolType::Relation,
            None,
            Some(Arc::new(Mutex::new(relation_data))),
        );

        self.current_relation_scope.pop();
        self.current_relation_locations.pop();
    }

    fn post_process_rel_root(&mut self, obj: &substrait::RelRoot) {
        let mut names = Vec::new();
        names.extend(obj.names.iter().cloned());

        let unique_name = self.symbol_table.get_unique_name(ROOT_NAMES);
        self.symbol_table.define_symbol(
            unique_name,
            self.current_location().field("rel_root"),
            SymbolType::Root,
            Some(Box::new(SourceType::Unknown)),
            Some(Arc::new(Mutex::new(names))),
        );
    }

    fn pre_process_read_rel(&mut self, obj: &substrait::ReadRel) {
        if let Some(base_schema) = &obj.base_schema {
            let name = self.symbol_table.get_unique_name("schema");
            let symbol = self.symbol_table.define_symbol(
                name,
                self.current_location().field("base_schema"),
                SymbolType::Schema,
                None,
                Some(Arc::new(Mutex::new(base_schema.clone()))),
            );
            self.read_relation_schemas.insert(
                self.current_relation_scope.last().unwrap().to_string(),
                symbol,
            );
            // Traverse the named struct to process its contents
            base_schema.traverse(self);
        }

        // Handle the read_type (source) variants
        if let Some(read_type) = &obj.read_type {
            use substrait::read_rel::ReadType;
            match read_type {
                ReadType::LocalFiles(local_files) => {
                    let name = self.symbol_table.get_unique_name("local");
                    let symbol = self.symbol_table.define_symbol(
                        name,
                        self.current_location().field("local_files"),
                        SymbolType::Source,
                        Some(Box::new(SourceType::LocalFiles)),
                        Some(Arc::new(Mutex::new(local_files.clone()))),
                    );
                    if let Some(scope) = self.current_relation_scope.last() {
                        self.read_relation_sources.insert(scope.to_string(), symbol);
                    }
                }
                ReadType::VirtualTable(virtual_table) => {
                    let name = self.symbol_table.get_unique_name("virtual");
                    let symbol = self.symbol_table.define_symbol(
                        name,
                        self.current_location().field("virtual_table"),
                        SymbolType::Source,
                        Some(Box::new(SourceType::VirtualTable)),
                        Some(Arc::new(Mutex::new(virtual_table.clone()))),
                    );
                    if let Some(scope) = self.current_relation_scope.last() {
                        self.read_relation_sources.insert(scope.to_string(), symbol);
                    }
                }
                ReadType::NamedTable(named_table) => {
                    let name = self.symbol_table.get_unique_name("named");
                    let symbol = self.symbol_table.define_symbol(
                        name,
                        self.current_location().field("named_table"),
                        SymbolType::Source,
                        Some(Box::new(SourceType::NamedTable)),
                        Some(Arc::new(Mutex::new(named_table.clone()))),
                    );
                    if let Some(scope) = self.current_relation_scope.last() {
                        self.read_relation_sources.insert(scope.to_string(), symbol);
                    }
                }
                ReadType::ExtensionTable(extension_table) => {
                    let name = self.symbol_table.get_unique_name("extensiontable");
                    let symbol = self.symbol_table.define_symbol(
                        name,
                        self.current_location().field("extension_table"),
                        SymbolType::Source,
                        Some(Box::new(SourceType::ExtensionTable)),
                        Some(Arc::new(Mutex::new(extension_table.clone()))),
                    );
                    if let Some(scope) = self.current_relation_scope.last() {
                        self.read_relation_sources.insert(scope.to_string(), symbol);
                    }
                }
                ReadType::IcebergTable(iceberg_table) => {
                    let name = self.symbol_table.get_unique_name("iceberg");
                    let symbol = self.symbol_table.define_symbol(
                        name,
                        self.current_location().field("iceberg_table"),
                        SymbolType::Source,
                        Some(Box::new(SourceType::IcebergTable)),
                        Some(Arc::new(Mutex::new(iceberg_table.clone()))),
                    );
                    if let Some(scope) = self.current_relation_scope.last() {
                        self.read_relation_sources.insert(scope.to_string(), symbol);
                    }
                }
            }
        }
    }

    /// Migrate old grouping format to new format when loading from binary.
    /// Old format: Grouping.grouping_expressions (deprecated)
    /// New format: AggregateRel.grouping_expressions + Grouping.expression_references
    #[allow(deprecated)]
    fn pre_process_aggregate_rel(&mut self, obj: &substrait::AggregateRel) {
        // Check if we need to migrate from old to new format
        // If AggregateRel.grouping_expressions is empty but Grouping.grouping_expressions is populated,
        // we need to migrate
        let needs_migration = obj.grouping_expressions.is_empty()
            && obj
                .groupings
                .iter()
                .any(|g| !g.grouping_expressions.is_empty());

        if needs_migration {
            // NOTE: We can't modify obj here since it's immutable. The migration will need to happen
            // in the parser when we reconstruct the relation from textplan.
            // For now, we'll use the deprecated field in add_grouping_to_relation.
            // The proper solution is to have the parser always output the new format.
        }
    }
}
