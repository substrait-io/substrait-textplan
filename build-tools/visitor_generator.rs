// SPDX-License-Identifier: Apache-2.0

// Visitor generator for Substrait protocol buffers.
// This module is designed to be used from the build.rs script to generate
// a complete visitor implementation for the Substrait protocol buffers.
// It recursively discovers all message types by parsing proto files.

use phf::phf_map;
use prost_types::{DescriptorProto, FieldDescriptorProto, FileDescriptorSet};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tinytemplate::TinyTemplate;

/// Helper function to check if a field is a Rel type.
///
/// Rel-typed fields should be visited last to ensure that expressions
/// (which may contain subqueries) are processed before child relations.
/// This ensures subquery schemas are created before main query schemas.
fn is_rel_field(field: &FieldDescriptorProto) -> bool {
    if let Some(type_name) = field.type_name.as_ref() {
        // Check if the type name ends with ".Rel" (like "substrait.Rel")
        // or is exactly ".substrait.Rel"
        type_name.ends_with(".Rel") || type_name == ".substrait.Rel"
    } else {
        false
    }
}

/// Error type for visitor generator operations
#[derive(Debug)]
pub enum GeneratorError {
    Io(io::Error),
}

impl fmt::Display for GeneratorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GeneratorError::Io(err) => write!(f, "I/O error: {}", err),
        }
    }
}

impl Error for GeneratorError {}

impl From<io::Error> for GeneratorError {
    fn from(err: io::Error) -> Self {
        GeneratorError::Io(err)
    }
}

/// The main visitor generator
pub struct VisitorGenerator {
    // Information on every message stored by source file.
    proto_descriptor: FileDescriptorSet,

    // Where to write the completed visitor.
    output_path: PathBuf,

    // Type registries
    message_types: HashMap<String, DescriptorProto>, // full_name -> MessageType

    // Categories of message types (for generating visitor methods)
    top_level_messages: BTreeSet<String>, // full_names of top-level messages (like Plan)
    all_message_types: BTreeSet<String>,  // full_names of all message types
}

#[derive(Serialize)]
struct MethodContext {
    indent: String,
    top_level: bool,
    name: String,
    type_path: String,
}

impl VisitorGenerator {
    /// Create a new generator
    pub fn new(proto_descriptor: FileDescriptorSet, output_path: impl AsRef<Path>) -> Self {
        Self {
            proto_descriptor,
            output_path: output_path.as_ref().to_path_buf(),
            message_types: HashMap::new(),
            top_level_messages: BTreeSet::new(),
            all_message_types: BTreeSet::new(),
        }
    }

    fn index_protomessage(&mut self, parent_name: &str, msg: &DescriptorProto) {
        let message_name = if parent_name.is_empty() {
            msg.name.clone().unwrap_or_default()
        } else {
            format!("{}.{}", parent_name, msg.name.clone().unwrap_or_default())
        };
        self.message_types.insert(message_name.clone(), msg.clone());
        self.all_message_types.insert(message_name.clone());
        for nested_message in &msg.nested_type {
            self.index_protomessage(&message_name, nested_message);
        }
    }

    fn index_protobuffers(&mut self) {
        for fileinfo in &self.proto_descriptor.file.clone() {
            let package = fileinfo.package.clone().unwrap_or_default();
            if package.starts_with("google.") {
                continue;
            }
            for msg in &fileinfo.message_type {
                self.index_protomessage(&package, msg)
            }
        }
    }

    /// Run the generator - main entry point
    pub fn run(&mut self) -> Result<(), GeneratorError> {
        // 1. Index the messages for later easy access.
        self.index_protobuffers();

        // 3. Mark special message types (mainly Plan as top-level)
        self.mark_special_message_types();

        // 4. Generate visitor code
        let visitor_code = self.generate_visitor_code()?;

        // 5. Write the generated code to the output file
        self.write_visitor_code(&visitor_code)?;

        Ok(())
    }

    /// Mark special message types
    fn mark_special_message_types(&mut self) {
        // Mark "Plan" as a top-level message
        for (full_name, message) in &self.message_types {
            let message_name = message.name().to_string();
            if message_name.ends_with("Plan") || message_name.ends_with("ExtendedExpression") {
                self.top_level_messages.insert(full_name.clone());
            }
        }
    }

    /// Generate visitor code for the Substrait protocol buffers
    fn generate_visitor_code(&mut self) -> Result<String, GeneratorError> {
        let mut output = String::new();

        // Generate file header with imports
        self.generate_header(&mut output);

        // Generate the base visitor trait
        self.generate_visitor_trait(&mut output);

        // Generate the traversable trait
        self.generate_traversable_trait(&mut output);

        Ok(output)
    }

    /// Generate file header with imports
    fn generate_header(&self, output: &mut String) {
        output.push_str(
            r#"
// SPDX-License-Identifier: Apache-2.0

//! GENERATED CODE - DO NOT MODIFY
//! Generated visitor for Substrait protocol buffers.

extern crate substrait;

use crate::textplan::common::ProtoLocation;
use substrait::proto;
use substrait::proto::extensions;

#[allow(deprecated,unused_variables,unreachable_patterns)]

"#,
        );
    }

    /// Generate the visitor trait
    fn generate_visitor_trait(&mut self, output: &mut String) {
        output.push_str(
            r#"
/// Base visitor trait for Substrait plans.
///
/// This trait defines the visit methods for all protobuf message types in the Substrait schema.
/// It's intended to be implemented by concrete visitors that need to traverse and process
/// Substrait plans.
///
/// The trait automatically tracks the current location in the protocol buffer structure
/// using ProtoLocation, which can be used for detailed error reporting and symbol resolution.
pub trait PlanProtoVisitor {
    /// Get the current location in the protocol buffer being visited
    fn current_location(&self) -> &ProtoLocation;

    /// Set the current location in the protocol buffer being visited
    fn set_location(&mut self, location: ProtoLocation);

    /// Create a location for a specific field of the current object
    fn field_location(&self, field_name: &str) -> ProtoLocation {
        self.current_location().field(field_name)
    }

    /// Create a location for an indexed field of the current object
    fn indexed_field_location(&self, field_name: &str, index: usize) -> ProtoLocation {
        self.current_location().indexed_field(field_name, index)
    }

"#,
        );

        output.push('\n');

        for full_name in &self.top_level_messages {
            if let Some(_message) = self.message_types.get(full_name) {
                self.generate_preprocess_method(full_name, output, 4);
            }
        }

        // Generate visit methods for all other message types
        // Use a queue to ensure we process all types that could be discovered
        let mut to_process: Vec<String> = self
            .all_message_types
            .iter()
            .filter(|name| !self.top_level_messages.contains(*name))
            .cloned()
            .collect();

        while let Some(full_name) = to_process.pop() {
            if let Some(_message) = self.message_types.get(&full_name) {
                self.generate_preprocess_method(&full_name, output, 4);
            }
        }

        output.push('\n');

        for full_name in &self.top_level_messages {
            if let Some(_message) = self.message_types.get(full_name) {
                self.generate_postprocess_method(full_name, output, 4);
            }
        }

        // Generate visit methods for all other message types
        // Use a queue to ensure we process all types that could be discovered
        let mut to_process: Vec<String> = self
            .all_message_types
            .iter()
            .filter(|name| !self.top_level_messages.contains(*name))
            .cloned()
            .collect();

        while let Some(full_name) = to_process.pop() {
            if let Some(_message) = self.message_types.get(&full_name) {
                self.generate_postprocess_method(&full_name, output, 4);
            }
        }

        // Close trait
        output.push_str("}\n\n");
    }

    /// Generate the traversable trait
    fn generate_traversable_trait(&mut self, output: &mut String) {
        output.push_str(
            r#"
// Define the traversable trait for each proto type
pub trait Traversable {
    fn traverse<V: PlanProtoVisitor>(&self, visitor: &mut V);
}
"#,
        );

        // Generate visit methods for top-level messages first
        for full_name in &self.top_level_messages {
            if let Some(message) = self.message_types.get(full_name) {
                self.generate_visit_method(full_name, message, output, 4);
            }
        }

        // Generate visit methods for all other message types
        // Use a queue to ensure we process all types that could be discovered
        let mut to_process: Vec<String> = self
            .all_message_types
            .iter()
            .filter(|name| !self.top_level_messages.contains(*name))
            .cloned()
            .collect();

        while let Some(full_name) = to_process.pop() {
            if let Some(message) = self.message_types.get(&full_name) {
                self.generate_visit_method(&full_name, message, output, 4);
            }
        }
    }

    /// Get the portion of a method name for a message type
    fn get_method_name_fragment(&self, message_name: &str) -> String {
        static NAME_OVERRIDES: phf::Map<&'static str, &'static str> = phf_map! {
            "substrait.Expression.Literal.Decimal" => "expression_literal_decimal",
            "substrait.Expression.Literal.List" => "expression_literal_list",
            "substrait.Expression.Literal.Map" => "expression_literal_map",
            "substrait.Expression.Literal.Struct" => "expression_literal_struct",
            "substrait.Expression.Literal.VarChar" => "expression_literal_varchar",
            "substrait.Expression.Literal.UserDefined" => "expression_literal_user_defined",
            "substrait.Expression.Subquery.Scalar" => "expression_subquery_scalar",
            "substrait.Expression.MaskExpression.MapSelect.MapKey" => "mask_expression_map_select_mapkey",
            "substrait.Expression.MaskExpression.ListSelect.ListSelectItem.ListElement" => "mask_expression_list_select_item_element",
            "substrait.Expression.Nested.Struct" => "expression_nested_struct",
            "substrait.Expression.Nested.Map.KeyValue" => "expression_nested_map_key_value",
            "substrait.Expression.Nested.Map" => "expression_nested_map",
            "substrait.Expression.Nested.List" => "expression_nested_list",
        };
        if message_name.starts_with(".") && NAME_OVERRIDES.contains_key(&message_name[1..]) {
            return NAME_OVERRIDES[&message_name[1..]].to_string();
        } else if NAME_OVERRIDES.contains_key(message_name) {
            return NAME_OVERRIDES[message_name].to_string();
        }
        let method_name = match message_name.rfind('.') {
            Some(pos) => &message_name[pos + 1..],
            None => message_name, // Return the entire string if no period is found
        };
        // Convert camel case to snake case for method name
        to_snake_case(method_name)
    }

    /// Get the Rust path for a message type
    fn get_rust_type_path(&self, full_name: &str) -> String {
        // Parse the package path to detect where this message is defined
        let mut package_parts: Vec<&str> = full_name.split('.').collect();
        let message_name = package_parts.pop().unwrap();

        // Use the full package path except the path we know.
        let mut fixed_package_parts: Vec<String> = package_parts[1..]
            .iter()
            .map(|&part| to_snake_case(part))
            .map(|part| to_rust_safe_name(&part))
            .collect();

        let package_name: &str;
        if !fixed_package_parts.is_empty() && fixed_package_parts[0] == "extensions" {
            fixed_package_parts = fixed_package_parts.split_off(1);
            package_name = "extensions";
        } else {
            package_name = "proto";
        }
        let intervening_packages = if fixed_package_parts.is_empty() {
            "".to_string()
        } else {
            fixed_package_parts.join("::") + "::"
        };

        format!(
            "{}::{}{}",
            package_name,
            intervening_packages,
            fix_pascal_case(message_name)
        )
    }

    /// Generate an implementation for a preprocess method
    fn generate_preprocess_method(&self, full_name: &str, output: &mut String, indent: usize) {
        let indent_str = " ".repeat(indent);
        let method_name = self.get_method_name_fragment(full_name);
        let rust_type_path = self.get_rust_type_path(full_name);

        static METHOD_TEMPLATE: &str = r#"
{-indent}fn pre_process_{name}(&mut self, obj: &{type_path}) \{}
"#;
        let mut tt = TinyTemplate::new();
        let result = tt.add_template("method", METHOD_TEMPLATE);
        if result.is_err() {
            panic!("{}", result.unwrap_err());
        }
        let context = MethodContext {
            indent: indent_str.clone(),
            top_level: full_name == "substrait.Plan" || full_name == "substrait.ExtendedExpression",
            name: method_name,
            type_path: rust_type_path.clone(),
        };

        let rendered = tt.render("method", &context);
        if rendered.is_err() {
            panic!("{}", rendered.unwrap_err());
        }
        output.push_str(&rendered.unwrap());
    }

    /// Generate an implementation for a postprocess method
    fn generate_postprocess_method(&self, full_name: &str, output: &mut String, indent: usize) {
        let indent_str = " ".repeat(indent);
        let method_name = self.get_method_name_fragment(full_name);
        let rust_type_path = self.get_rust_type_path(full_name);

        static METHOD_TEMPLATE: &str = r#"
{-indent}fn post_process_{name}(&mut self, obj: &{type_path}) \{}
"#;
        let mut tt = TinyTemplate::new();
        let result = tt.add_template("method", METHOD_TEMPLATE);
        if result.is_err() {
            panic!("{}", result.unwrap_err());
        }
        let context = MethodContext {
            indent: indent_str.clone(),
            top_level: full_name == "substrait.Plan" || full_name == "substrait.ExtendedExpression",
            name: method_name,
            type_path: rust_type_path.clone(),
        };

        let rendered = tt.render("method", &context);
        if rendered.is_err() {
            panic!("{}", rendered.unwrap_err());
        }
        output.push_str(&rendered.unwrap());
    }

    /// Get fields in the correct traversal order.
    ///
    /// Fields are ordered so that Rel-typed fields come last. This ensures that
    /// expressions (which may contain subqueries) are processed before child relations,
    /// so subquery schemas are created before main query schemas.
    fn get_ordered_fields<'a>(
        &self,
        _full_name: &str,
        message: &'a DescriptorProto,
    ) -> Vec<&'a FieldDescriptorProto> {
        let mut non_rel_fields = Vec::new();
        let mut rel_fields = Vec::new();

        for field in &message.field {
            if is_rel_field(field) {
                rel_fields.push(field);
            } else {
                non_rel_fields.push(field);
            }
        }

        // Return non-Rel fields first, then Rel fields
        non_rel_fields.extend(rel_fields);
        non_rel_fields
    }

    /// Generate an implementation for a visit method
    fn generate_visit_method(
        &self,
        full_name: &str,
        message: &DescriptorProto,
        output: &mut String,
        indent: usize,
    ) {
        let indent_str = " ".repeat(indent);
        let method_name = self.get_method_name_fragment(full_name);
        let rust_type_path = self.get_rust_type_path(full_name);

        static METHOD_TEMPLATE: &str = r#"
impl Traversable for {type_path} \{
{indent}fn traverse<V: PlanProtoVisitor>(&self, visitor: &mut V) \{
{indent}    // Save current location
{indent}    let prev_location = visitor.current_location().clone();

{{- if top_level }}
{indent}    // Set location to this object for the first time.
{indent}    visitor.set_location(ProtoLocation::new(self));
{{ endif }}
{indent}    visitor.pre_process_{name}(self);

"#;
        let mut tt = TinyTemplate::new();
        let result = tt.add_template("method", METHOD_TEMPLATE);
        if result.is_err() {
            panic!("{}", result.unwrap_err());
        }
        let context = MethodContext {
            indent: indent_str.clone(),
            top_level: full_name == "substrait.Plan" || full_name == "substrait.ExtendedExpression",
            name: method_name.clone(),
            type_path: rust_type_path.clone(),
        };

        let rendered = tt.render("method", &context);
        if rendered.is_err() {
            panic!("{}", rendered.unwrap_err());
        }
        output.push_str(&rendered.unwrap());

        // Get fields in the correct traversal order (Rel fields last)
        let ordered_fields = self.get_ordered_fields(full_name, message);

        // Output
        let mut handled_oneofs = HashSet::new();
        for rel_msg in ordered_fields {
            if let Some(oneof_index) = rel_msg.oneof_index {
                if handled_oneofs.contains(&oneof_index) {
                    continue;
                }
                self.generate_oneof_section(
                    message,
                    oneof_index,
                    &indent_str,
                    &to_snake_case(&rust_type_path),
                    output,
                );
                handled_oneofs.insert(oneof_index);
                continue;
            }

            if rel_msg.type_name().is_empty()
                || !self.all_message_types.contains(&rel_msg.type_name()[1..])
            {
                // This isn't a message we know about.
                continue;
            }
            if rel_msg.label() == prost_types::field_descriptor_proto::Label::Repeated {
                let field_name = to_rust_safe_name(rel_msg.name());
                output.push_str(&format!(
                    "{}    // Process repeated field: {}\n",
                    indent_str, field_name
                ));
                output.push_str(&format!(
                    "{}    for (idx, x) in self.{}.iter().enumerate() {{\n",
                    indent_str, field_name
                ));
                output.push_str(&format!(
                    "{}        // Set location to indexed field\n",
                    indent_str
                ));
                output.push_str(&format!(
                    "{}        let field_loc = visitor.indexed_field_location(\"{}\", idx);\n",
                    indent_str, field_name
                ));
                output.push_str(&format!(
                    "{}        visitor.set_location(field_loc);\n",
                    indent_str
                ));
                output.push_str(&format!("{}        x.traverse(visitor);\n", indent_str));
                output.push_str(&format!(
                    "{}        visitor.set_location(prev_location.clone());\n",
                    indent_str
                ));
                output.push_str(&format!("{}    }}\n", indent_str));
            } else if rel_msg.label() == prost_types::field_descriptor_proto::Label::Optional {
                let field_name = to_rust_safe_name(rel_msg.name());
                output.push_str(&format!(
                    "{}    // Process optional field: {}\n",
                    indent_str, field_name
                ));
                output.push_str(&format!(
                    "{}    if let Some({}) = &self.{} {{\n",
                    indent_str, field_name, field_name
                ));
                output.push_str(&format!("{}        // Set location to field\n", indent_str));
                output.push_str(&format!(
                    "{}        let field_loc = visitor.field_location(\"{}\");\n",
                    indent_str, field_name
                ));
                output.push_str(&format!(
                    "{}        visitor.set_location(field_loc);\n",
                    indent_str
                ));
                output.push_str(&format!(
                    "{}        {}.traverse(visitor);\n",
                    indent_str, field_name
                ));
                output.push_str(&format!(
                    "{}        visitor.set_location(prev_location.clone());\n",
                    indent_str
                ));
                output.push_str(&format!("{}    }}\n", indent_str));
            } else {
                let field_name = to_rust_safe_name(rel_msg.name());
                output.push_str(&format!(
                    "{}    // Process field: {}\n",
                    indent_str, field_name
                ));
                output.push_str(&format!(
                    "{}    let field_loc = visitor.field_location(\"{}\");\n",
                    indent_str, field_name
                ));
                output.push_str(&format!(
                    "{}    visitor.set_location(field_loc);\n",
                    indent_str
                ));
                output.push_str(&format!(
                    "{}        {}.traverse(visitor);\n",
                    indent_str, field_name
                ));

                output.push_str(&format!(
                    "{}        visitor.set_location(prev_location.clone());\n",
                    indent_str
                ));
            }
        }

        output.push_str(&format!(
            "\n{}    visitor.post_process_{}(self);\n",
            indent_str, method_name,
        ));

        // Restore the original location
        output.push_str(&format!(
            "\n{}    // Restore previous location\n",
            indent_str
        ));
        output.push_str(&format!(
            "{}    visitor.set_location(prev_location);\n",
            indent_str
        ));
        output.push_str(&format!("{}}}\n}}\n\n", indent_str));
    }

    /// Write the generated code to a file
    fn write_visitor_code(&self, code: &str) -> Result<(), GeneratorError> {
        let mut file = File::create(&self.output_path)?;
        file.write_all(code.as_bytes())?;
        Ok(())
    }

    fn generate_oneof_section(
        &self,
        message: &DescriptorProto,
        oneof_index: i32,
        indent_str: &String,
        parent_path: &String,
        output: &mut String,
    ) {
        // If not all items in this oneof are real messages, don't bother visiting.
        for rel_msg in &message.field {
            if rel_msg.type_name().is_empty()
                || !self.all_message_types.contains(&rel_msg.type_name()[1..])
            {
                // This isn't a message we know about.
                return;
            }
        }

        let oneof_name = message.oneof_decl[oneof_index as usize].name();
        let oneof_type_name = to_pascal_case(oneof_name);
        output.push_str(&format!(
            "{}    if let Some({}) = &self.{} {{\n",
            indent_str,
            to_rust_safe_name(oneof_name),
            to_rust_safe_name(oneof_name)
        ));
        output.push_str(&format!(
            "{}        match {} {{\n",
            indent_str,
            to_rust_safe_name(oneof_name)
        ));

        for rel_msg in &message.field {
            if rel_msg.oneof_index != Some(oneof_index) {
                continue;
            }
            let rel_variant = to_pascal_case(rel_msg.name());

            output.push_str(&format!(
                "{}            {}::{}::{}(value) => {{\n",
                indent_str, parent_path, oneof_type_name, rel_variant
            ));
            output.push_str(&format!(
                "{}                // Set location to field within oneof\n",
                indent_str
            ));
            output.push_str(&format!(
                "{}                let field_loc = visitor.field_location(\"{}\");\n",
                indent_str,
                to_rust_safe_name(rel_msg.name())
            ));
            output.push_str(&format!(
                "{}                visitor.set_location(field_loc);\n",
                indent_str
            ));
            output.push_str(&format!(
                "{}                value.traverse(visitor);\n",
                indent_str
            ));
            output.push_str(&format!(
                "{}                visitor.set_location(prev_location.clone());\n",
                indent_str
            ));
            output.push_str(&format!("{}            }}\n", indent_str));
        }
        // Default case
        output.push_str(&format!("{}            _ => {{}}\n", indent_str));

        output.push_str(&format!("{}        }}\n", indent_str));
        output.push_str(&format!("{}    }}\n", indent_str));
    }
}

/// Convert CamelCase to snake_case
fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    let mut prev_is_lowercase = false;
    let mut i = 0;
    let chars: Vec<char> = s.chars().collect();

    while i < chars.len() {
        let c = chars[i];

        // Skip adding underscores around :: sequences
        if c == ':' && i + 1 < chars.len() && chars[i + 1] == ':' {
            result.push(':');
            result.push(':');
            i += 2; // Skip both ':' characters
            prev_is_lowercase = false;
            continue;
        }

        if c.is_uppercase() {
            if i > 0 && prev_is_lowercase {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap());
            prev_is_lowercase = false;
        } else {
            result.push(c);
            prev_is_lowercase = c.is_lowercase() && c != ':';
        }

        i += 1;
    }

    result
}

fn to_rust_safe_name(s: &str) -> String {
    match s {
        "type" | "enum" | "struct" | "match" | "if" | "else" => format!("r#{}", s),
        _ => s.to_string(),
    }
}

/// Convert acronyms in CamelCase to proper Pascal case
/// e.g., SimpleExtensionURI -> SimpleExtensionUri
fn fix_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut i = 0;
    let chars: Vec<char> = s.chars().collect();

    while i < chars.len() {
        let c = chars[i];
        result.push(c);

        // Check for acronyms (consecutive uppercase)
        if c.is_uppercase() && i + 1 < chars.len() && chars[i + 1].is_uppercase() {
            // Found start of an acronym, find its end
            let start = i;
            i += 1;
            while i < chars.len() && chars[i].is_uppercase() {
                i += 1;
            }

            // Fix the acronym (except first letter which remains uppercase)
            for j in start + 1..i {
                if j < chars.len() {
                    result.push(chars[j].to_lowercase().next().unwrap());
                }
            }

            // Adjust i since we've processed these characters
            i -= 1;
        }

        i += 1;
    }

    result
}

/// Convert underscore separated strings to PascalCase.
fn to_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;

    for c in s.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }

    result
}

/// Run the visitor generator
pub fn generate(proto_dir: FileDescriptorSet, output_path: &Path) -> Result<(), Box<dyn Error>> {
    println!("cargo:warning=Generating visitor code for Substrait protobuf schema...");

    let mut generator = VisitorGenerator::new(proto_dir, output_path);
    generator
        .run()
        .map_err(|e| Box::<dyn Error>::from(format!("Generator error: {}", e)))?;

    println!(
        "cargo:warning=Generated visitor code successfully at {}",
        output_path.display()
    );

    Ok(())
}
