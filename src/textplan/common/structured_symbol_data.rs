// Used by the PlanRelation and Relation concepts to track connectivity.
use crate::textplan::SymbolInfo;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct RelationData {
    // Keeps track of the first node in a pipeline. For relations starting a
    // pipeline this will not be a self-reference -- it will be None unless
    // it is in another pipeline (which in that case the value will be the node
    // that starts that pipeline). As such this will only have None as a value
    // when it is a root node.
    pub pipeline_start: Option<Arc<SymbolInfo>>,
    // The next node in the pipeline that this node is part of.
    pub continuing_pipeline: Option<Arc<SymbolInfo>>,
    // The next nodes in the pipelines that this node starts.
    pub new_pipelines: Vec<Arc<SymbolInfo>>,
    // Expressions in this relation consume subqueries with these symbols.
    pub sub_query_pipelines: Vec<Arc<SymbolInfo>>,
    // The information corresponding to the relation without any references to
    // other relations or inputs.
    pub relation: substrait::proto::Rel,
    // Source stores the input symbol of a read relation.
    pub source: Option<Arc<SymbolInfo>>,
    // Schema keeps track schema used in this relation.
    pub schema: Option<Arc<SymbolInfo>>,
    // Fallback schema name if schema symbol wasn't found during parsing (to be resolved later).
    pub schema_name: Option<String>,
    // Column name for each field known to this relation (in field order). Used
    // to determine what fields are coming in as well and fields are going out.
    pub field_references: Vec<Arc<SymbolInfo>>,
    // Each field reference here was generated within the current relation.
    pub generated_field_references: Vec<Arc<SymbolInfo>>,
    // Local aliases for field references in this relation. Used to replace the
    // normal form symbols would take for this relation's use only. (Later
    // references to the symbol would use the alias.)
    pub generated_field_reference_alternative_expression: HashMap<usize, String>,
    // Temporary storage for global aliases for expressions. Used during the
    // construction of a relation.
    pub generated_field_reference_aliases: HashMap<usize, String>,
    // If populated, supersedes the combination of fieldReferences and
    // generatedFieldReferences for the field symbols exposed by this relation.
    pub output_field_references: Vec<Arc<SymbolInfo>>,
    // Contains the field reference names seen so far while processing this
    // relation along with the id of the first occurrence. Used to detect when
    // fully qualified references are necessary.
    pub seen_field_reference_names: HashMap<String, usize>,
    // Root output names (for root relations only).
    pub root_names: Vec<String>,
}

impl RelationData {
    // Basic constructor
    pub fn new(relation: substrait::proto::Rel) -> Self {
        RelationData {
            pipeline_start: None,
            continuing_pipeline: None,
            new_pipelines: Vec::new(),
            sub_query_pipelines: Vec::new(),
            relation,
            source: None,
            schema: None,
            schema_name: None,
            field_references: Vec::new(),
            generated_field_references: Vec::new(),
            generated_field_reference_alternative_expression: HashMap::new(),
            generated_field_reference_aliases: HashMap::new(),
            output_field_references: Vec::new(),
            seen_field_reference_names: HashMap::new(),
            root_names: Vec::new(),
        }
    }

    // Constructor for root markers (no actual Rel protobuf, just connectivity)
    pub fn new_empty() -> Self {
        RelationData {
            pipeline_start: None,
            continuing_pipeline: None,
            new_pipelines: Vec::new(),
            sub_query_pipelines: Vec::new(),
            relation: substrait::proto::Rel::default(),
            source: None,
            schema: None,
            schema_name: None,
            field_references: Vec::new(),
            generated_field_references: Vec::new(),
            generated_field_reference_alternative_expression: HashMap::new(),
            generated_field_reference_aliases: HashMap::new(),
            output_field_references: Vec::new(),
            seen_field_reference_names: HashMap::new(),
            root_names: Vec::new(),
        }
    }
}

// Used by Schema symbols to keep track of assigned values.
#[derive(Debug, Clone)]
pub struct SchemaData {
    anchor_reference: u32,
}

impl SchemaData {
    pub(crate) fn new(anchor_reference: u32) -> Self {
        SchemaData { anchor_reference }
    }
}

// Used by Extension Space symbols to keep track of assigned values.
#[derive(Debug, Clone)]
pub struct ExtensionSpaceData {
    anchor_reference: u32,
}

impl ExtensionSpaceData {
    pub(crate) fn new(anchor_reference: u32) -> Self {
        ExtensionSpaceData { anchor_reference }
    }

    pub fn anchor_reference(&self) -> u32 {
        self.anchor_reference
    }
}

// Used by Function symbols to keep track of the name and assigned anchors.
#[derive(Debug, Clone)]
pub struct FunctionData {
    pub name: String,
    pub extension_uri_reference: Option<u32>,
    pub anchor: u32,
}

impl FunctionData {
    pub(crate) fn new(name: String, extension_uri_reference: Option<u32>, anchor: u32) -> Self {
        FunctionData {
            name,
            extension_uri_reference,
            anchor,
        }
    }
}
