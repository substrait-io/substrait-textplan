// SPDX-License-Identifier: Apache-2.0

//! Visitor interface for traversing a Substrait plan.
//!
//! This module provides a simplified implementation of the visitor pattern
//! for traversing parsed textplan data.

use std::any::Any;
use std::sync::Arc;

use crate::textplan::common::location::{BoxedLocation, Location};
use crate::textplan::parser::error_listener::ErrorListener;
use crate::textplan::symbol_table::{RelationType, SymbolInfo, SymbolTable, SymbolType};

/// Simplified representation of a parse node type
#[derive(Debug, Clone, PartialEq)]
pub enum NodeType {
    Plan,
    PlanDetail,
    Relation,
    Schema,
    Source,
    Root,
    ExtensionSpace,
    Function,
    Expression,
    Constant,
    Identifier,
    Other(String),
}

/// Simplified representation of a parse node
#[derive(Debug, Clone)]
pub struct ParseNode {
    /// The type of node
    node_type: NodeType,
    /// The text content of the node
    text: String,
    /// The location in the source text
    location: BoxedLocation,
    /// Any child nodes
    children: Vec<ParseNode>,
}

impl ParseNode {
    /// Creates a new parse node
    pub fn new<L: Location + 'static>(node_type: NodeType, text: String, location: L) -> Self {
        Self {
            node_type,
            text,
            location: Box::new(location),
            children: Vec::new(),
        }
    }

    /// Adds a child node
    pub fn add_child(&mut self, child: ParseNode) {
        self.children.push(child);
    }

    /// Returns the node type
    pub fn node_type(&self) -> &NodeType {
        &self.node_type
    }

    /// Returns the text content
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns a reference to the location
    pub fn location(&self) -> &BoxedLocation {
        &self.location
    }

    /// Returns the children
    pub fn children(&self) -> &[ParseNode] {
        &self.children
    }
}

/// Visitor interface for traversing parse nodes
pub trait Visitor {
    /// Visits a parse node
    fn visit(&mut self, node: &ParseNode) -> Option<Box<dyn Any>>;
}

/// Implementation of a visitor for building a symbol table
pub struct SymbolTableVisitor {
    symbol_table: SymbolTable,
    error_listener: ErrorListener,
    current_relation_scope: Option<Arc<SymbolInfo>>,
    num_spaces_seen: usize,
    num_functions_seen: usize,
}

impl SymbolTableVisitor {
    /// Creates a new SymbolTableVisitor
    pub fn new(symbol_table: SymbolTable, error_listener: ErrorListener) -> Self {
        Self {
            symbol_table,
            error_listener,
            current_relation_scope: None,
            num_spaces_seen: 0,
            num_functions_seen: 0,
        }
    }

    /// Returns a reference to the symbol table
    pub fn symbol_table(&self) -> &SymbolTable {
        &self.symbol_table
    }

    /// Returns a reference to the error listener
    pub fn error_listener(&self) -> &ErrorListener {
        &self.error_listener
    }

    /// Handles a relation node
    fn handle_relation(&mut self, node: &ParseNode) -> Option<Box<dyn Any>> {
        // Extract relation type
        let relation_type = self.extract_relation_type(node);

        // Convert relation type string to enum
        let relation_subtype = match relation_type.as_str() {
            "read" => RelationType::Read,
            "project" => RelationType::Project,
            "join" => RelationType::Join,
            "cross" => RelationType::Cross,
            "fetch" => RelationType::Fetch,
            "aggregate" => RelationType::Aggregate,
            "sort" => RelationType::Sort,
            "filter" => RelationType::Filter,
            "set" => RelationType::Set,
            "hash_join" => RelationType::HashJoin,
            "merge_join" => RelationType::MergeJoin,
            "exchange" => RelationType::Exchange,
            "ddl" => RelationType::Ddl,
            "write" => RelationType::Write,
            "extension_leaf" => RelationType::ExtensionLeaf,
            "extension_single" => RelationType::ExtensionSingle,
            "extension_multi" => RelationType::ExtensionMulti,
            _ => RelationType::Unknown,
        };

        // Extract relation name
        let relation_name = self.extract_relation_name(node);

        // Create symbol for relation
        let subtype_box: Box<dyn Any + Send + Sync> = Box::new(relation_subtype);
        let symbol = self.symbol_table.define_symbol(
            relation_name,
            node.location(),
            SymbolType::Relation,
            Some(subtype_box),
            None,
        );

        // Set as current relation scope for processing details
        self.current_relation_scope = Some(symbol.clone());

        // Process relation details
        for child in node.children() {
            self.visit(child);
        }

        // Clear current relation scope
        self.current_relation_scope = None;

        Some(Box::new(symbol))
    }

    /// Extracts the relation type from a relation node
    fn extract_relation_type(&self, node: &ParseNode) -> String {
        // Find the first child with type Identifier that represents the relation type
        for child in node.children() {
            if let NodeType::Identifier = child.node_type() {
                return child.text().to_lowercase();
            }
        }
        "unknown".to_string()
    }

    /// Extracts the relation name from a relation node
    fn extract_relation_name(&self, node: &ParseNode) -> String {
        // In a real implementation, we would have better parsing
        // For now, just use the text of the node as a fallback
        node.text()
            .split_whitespace()
            .nth(2) // Skip "RELATION_TYPE RELATION "
            .unwrap_or("unknown_relation")
            .trim_matches(|c| c == '{' || c == '}')
            .to_string()
    }

    /// Handles a schema node
    fn handle_schema(&mut self, node: &ParseNode) -> Option<Box<dyn Any>> {
        // Extract schema name
        let schema_name = self.extract_schema_name(node);

        // Create symbol for schema
        let symbol = self.symbol_table.define_symbol(
            schema_name,
            node.location(),
            SymbolType::Schema,
            None,
            None,
        );

        // Process schema columns
        for child in node.children() {
            if let NodeType::Other(ref type_name) = child.node_type() {
                if type_name == "schema_column" {
                    self.handle_schema_column(child, &symbol);
                }
            }
        }

        Some(Box::new(symbol))
    }

    /// Extracts the schema name from a schema node
    fn extract_schema_name(&self, node: &ParseNode) -> String {
        // In a real implementation, we would have better parsing
        // For now, just use the text of the node as a fallback
        node.text()
            .split_whitespace()
            .nth(1) // Skip "schema "
            .unwrap_or("unknown_schema")
            .trim_matches(|c| c == '{' || c == '}')
            .to_string()
    }

    /// Handles a schema column node
    fn handle_schema_column(
        &mut self,
        node: &ParseNode,
        _schema: &Arc<SymbolInfo>,
    ) -> Option<Box<dyn Any>> {
        // Extract column name and type
        let parts: Vec<&str> = node.text().split_whitespace().collect();
        if parts.len() >= 2 {
            let column_name = parts[0].trim_end_matches(';').to_string();
            let column_type = parts[1].trim_end_matches(';').to_string();

            // Create symbol for column
            let column_symbol = self.symbol_table.define_symbol(
                column_name,
                node.location(),
                SymbolType::SchemaColumn,
                Some(Box::new(column_type)),
                None,
            );

            Some(Box::new(column_symbol))
        } else {
            None
        }
    }

    /// Handles a source node
    fn handle_source(&mut self, node: &ParseNode) -> Option<Box<dyn Any>> {
        // Extract source type and name
        let parts: Vec<&str> = node.text().split_whitespace().collect();
        if parts.len() >= 3 {
            let source_type = parts[1].to_string();
            let source_name = parts[2].trim_matches(|c| c == '{' || c == '}').to_string();

            // Create symbol for source
            let source_symbol = self.symbol_table.define_symbol(
                source_name,
                node.location(),
                SymbolType::Source,
                Some(Box::new(source_type)),
                None,
            );

            Some(Box::new(source_symbol))
        } else {
            None
        }
    }

    /// Handles a root node
    fn handle_root(&mut self, node: &ParseNode) -> Option<Box<dyn Any>> {
        // Create symbol for root
        let root_symbol = self.symbol_table.define_symbol(
            "root".to_string(),
            node.location(),
            SymbolType::Root,
            None,
            None,
        );

        Some(Box::new(root_symbol))
    }
}

impl Visitor for SymbolTableVisitor {
    fn visit(&mut self, node: &ParseNode) -> Option<Box<dyn Any>> {
        match node.node_type() {
            NodeType::Plan => {
                // Process all children of the plan
                for child in node.children() {
                    self.visit(child);
                }
                None
            }
            NodeType::Relation => self.handle_relation(node),
            NodeType::Schema => self.handle_schema(node),
            NodeType::Source => self.handle_source(node),
            NodeType::Root => self.handle_root(node),
            NodeType::ExtensionSpace => {
                // Process the extension space
                self.num_spaces_seen += 1;
                let space_name = format!("space{}", self.num_spaces_seen);

                // Create a symbol for the extension space
                let space_symbol = self.symbol_table.define_symbol(
                    space_name,
                    node.location(),
                    SymbolType::ExtensionSpace,
                    None,
                    None,
                );

                // Process all children
                for child in node.children() {
                    self.visit(child);
                }

                Some(Box::new(space_symbol))
            }
            NodeType::Function => {
                // Process the function
                self.num_functions_seen += 1;

                // Get the function name (either from a child or generate one)
                let function_name = if let Some(name_node) = node
                    .children()
                    .iter()
                    .find(|child| matches!(child.node_type(), NodeType::Identifier))
                {
                    name_node.text().to_string()
                } else {
                    format!("function{}", self.num_functions_seen)
                };

                // Create a symbol for the function
                let function_symbol = self.symbol_table.define_symbol(
                    function_name,
                    node.location(),
                    SymbolType::Function,
                    None,
                    None,
                );

                Some(Box::new(function_symbol))
            }
            // Add other node types as needed
            _ => {
                // Default behavior: visit all children
                for child in node.children() {
                    self.visit(child);
                }
                None
            }
        }
    }
}
