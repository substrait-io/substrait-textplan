// SPDX-License-Identifier: Apache-2.0

//! The proto_location module provides a way to track positions in protobuffer structures.

use crate::textplan::common::Location;
use std::any::Any;
use std::any::TypeId;
use std::fmt;
use std::hash::{Hash, Hasher};

/// Represents a path component in a nested protobuf structure
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathComponent {
    /// Represents a named field within a message
    Field(String),
    /// Represents an indexed element within a repeated field
    IndexedField(String, usize),
}

impl fmt::Display for PathComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathComponent::Field(name) => write!(f, ".{}", name),
            PathComponent::IndexedField(name, index) => write!(f, ".{}[{}]", name, index),
        }
    }
}

/// Represents a location in a protobuffer message.
///
/// This is used to track references to objects within the protobuffer structure,
/// allowing the system to maintain references to specific elements during visitor traversal.
#[derive(Debug, Clone, Eq)]
pub struct ProtoLocation {
    /// The type ID of the root protobuf message
    root_type_id: TypeId,
    /// A unique identifier for the root instance within the protobuf graph
    root_instance_id: u64,
    /// Path to the current element within the protobuf structure
    path: Vec<PathComponent>,
}

impl ProtoLocation {
    /// A special location used to indicate an unknown position.
    pub fn unknown() -> Self {
        Self {
            root_type_id: TypeId::of::<()>(),
            root_instance_id: 0,
            path: Vec::new(),
        }
    }

    /// Creates a new location from a protobuffer object.
    pub fn new<T: Any>(proto_obj: &T) -> Self {
        let root_type_id = TypeId::of::<T>();
        let root_instance_id = proto_obj as *const T as u64;

        Self {
            root_type_id,
            root_instance_id,
            path: Vec::new(),
        }
    }

    /// Returns the type ID of the root protobuf message.
    pub fn root_type_id(&self) -> TypeId {
        self.root_type_id
    }

    /// Returns the instance ID of the root protobuf message.
    pub fn root_instance_id(&self) -> u64 {
        self.root_instance_id
    }

    /// Returns the path components for this location
    pub fn path(&self) -> &[PathComponent] {
        &self.path
    }

    /// Create a new ProtoLocation that includes a field path component
    pub fn field(&self, field_name: &str) -> Self {
        let mut new_location = self.clone();
        new_location
            .path
            .push(PathComponent::Field(field_name.to_string()));
        new_location
    }

    /// Create a new ProtoLocation that includes an indexed field path component
    ///
    /// # Panics
    ///
    /// Panics if field_name is empty, as that would be invalid in a protobuf message.
    pub fn indexed_field(&self, field_name: &str, index: usize) -> Self {
        assert!(
            !field_name.is_empty(),
            "Field name cannot be empty for indexed field"
        );

        let mut new_location = self.clone();
        new_location
            .path
            .push(PathComponent::IndexedField(field_name.to_string(), index));
        new_location
    }

    /// Returns the full path string representation
    pub fn path_string(&self) -> String {
        if self.path.is_empty() {
            return "".to_string();
        }

        let mut result = String::new();
        for component in &self.path {
            result.push_str(&component.to_string());
        }
        result
    }
}

impl Location for ProtoLocation {
    fn is_unknown(&self) -> bool {
        // Compare with TypeId::of::<()>() can't be used in const contexts
        // so we use instance_id and empty path instead
        self.root_instance_id == 0 && self.path.is_empty()
    }

    // Implement a custom hash function specifically for ProtoLocation
    fn location_hash(&self) -> u64 {
        // Use a simpler hash combining technique
        let mut hash = 17u64;
        hash = hash.wrapping_mul(31).wrapping_add(self.root_instance_id);

        // Add hash contributions from the path
        for component in &self.path {
            match component {
                PathComponent::Field(name) => {
                    hash = hash.wrapping_mul(31).wrapping_add(1); // 1 for Field variant

                    // Simple hash of the string name
                    let name_hash = name
                        .as_bytes()
                        .iter()
                        .fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(*b as u64));
                    hash = hash.wrapping_mul(31).wrapping_add(name_hash);
                }
                PathComponent::IndexedField(name, index) => {
                    hash = hash.wrapping_mul(31).wrapping_add(2); // 2 for IndexedField variant

                    // Simple hash of the string name
                    let name_hash = name
                        .as_bytes()
                        .iter()
                        .fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(*b as u64));
                    hash = hash.wrapping_mul(31).wrapping_add(name_hash);
                    hash = hash.wrapping_mul(31).wrapping_add(*index as u64);
                }
            }
        }

        hash
    }

    // Create a boxed clone of this location
    fn box_clone(&self) -> Box<dyn Location> {
        Box::new(self.clone())
    }

    // Convert to Any for downcasting
    fn as_any(&self) -> &dyn Any {
        self
    }

    // Custom equals implementation for comparing with other locations
    fn equals(&self, other: &dyn Location) -> bool {
        // First check if other is a ProtoLocation using Any
        if let Some(other_proto) = other.as_any().downcast_ref::<ProtoLocation>() {
            // Compare proto locations directly
            self == other_proto
        } else {
            // Different types, so not equal
            false
        }
    }
}

impl PartialEq for ProtoLocation {
    fn eq(&self, other: &Self) -> bool {
        self.root_type_id == other.root_type_id
            && self.root_instance_id == other.root_instance_id
            && self.path == other.path
    }
}

impl Hash for ProtoLocation {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.root_type_id.hash(state);
        self.root_instance_id.hash(state);
        self.path.hash(state);
    }
}

impl fmt::Display for ProtoLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_unknown() {
            write!(f, "unknown proto location")
        } else if self.path.is_empty() {
            write!(
                f,
                "proto type {:?} instance 0x{:x}",
                self.root_type_id, self.root_instance_id
            )
        } else {
            write!(
                f,
                "proto type {:?} instance 0x{:x}{}",
                self.root_type_id,
                self.root_instance_id,
                self.path_string()
            )
        }
    }
}

impl Default for ProtoLocation {
    fn default() -> Self {
        Self::unknown()
    }
}

/// Macro for easily creating a ProtoLocation from a protobuf message
#[macro_export]
macro_rules! proto_location {
    ($obj:expr) => {
        $crate::textplan::common::proto_location::ProtoLocation::new(&$obj)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::textplan::common::unknown_location::UnknownLocation;
    use std::collections::hash_map::DefaultHasher;

    #[test]
    fn test_path_component_display() {
        // Test Field display
        let field = PathComponent::Field("message".to_string());
        assert_eq!(field.to_string(), ".message");

        // Test IndexedField display
        let indexed_field = PathComponent::IndexedField("repeated_field".to_string(), 5);
        assert_eq!(indexed_field.to_string(), ".repeated_field[5]");
    }

    #[test]
    fn test_proto_location_unknown() {
        let unknown = ProtoLocation::unknown();
        assert!(unknown.is_unknown());
        assert_eq!(unknown.root_instance_id(), 0);
        assert!(unknown.path().is_empty());
    }

    #[test]
    fn test_proto_location_default() {
        let default = ProtoLocation::default();
        assert!(default.is_unknown());
        assert_eq!(default.to_string(), "unknown proto location");
    }

    #[test]
    fn test_proto_location_new() {
        // Using a simple struct for testing
        #[derive(Debug)]
        struct TestProto {
            field1: String,
            field2: i32,
        }

        let test_proto = TestProto {
            field1: "test".to_string(),
            field2: 42,
        };

        let location = ProtoLocation::new(&test_proto);
        assert!(!location.is_unknown());
        assert_eq!(location.root_type_id(), TypeId::of::<TestProto>());
        assert_eq!(
            location.root_instance_id(),
            &test_proto as *const TestProto as u64
        );
        assert!(location.path().is_empty());
    }

    #[test]
    fn test_proto_location_field_and_indexed_field() {
        // Using a simple struct for testing
        #[derive(Debug)]
        struct TestProto {
            field1: String,
        }

        let test_proto = TestProto {
            field1: "test".to_string(),
        };

        let base_location = ProtoLocation::new(&test_proto);

        // Test field method
        let field_location = base_location.field("field1");
        assert_eq!(field_location.path().len(), 1);
        assert_eq!(
            field_location.path()[0],
            PathComponent::Field("field1".to_string())
        );

        // Test indexed_field method
        let indexed_location = base_location.indexed_field("repeated_field", 3);
        assert_eq!(indexed_location.path().len(), 1);
        assert_eq!(
            indexed_location.path()[0],
            PathComponent::IndexedField("repeated_field".to_string(), 3)
        );

        // Test chaining methods
        let nested_location = base_location
            .field("nested_message")
            .indexed_field("repeated_field", 2)
            .field("nested_field");

        assert_eq!(nested_location.path().len(), 3);
        assert_eq!(
            nested_location.path_string(),
            ".nested_message.repeated_field[2].nested_field"
        );
    }

    #[test]
    fn test_proto_location_path_string() {
        // Using a simple struct for testing
        #[derive(Debug)]
        struct TestProto {}

        let test_proto = TestProto {};
        let location = ProtoLocation::new(&test_proto);

        // Empty path
        assert_eq!(location.path_string(), "");

        // Single field
        let location_with_field = location.field("test_field");
        assert_eq!(location_with_field.path_string(), ".test_field");

        // Multiple components
        let complex_location = location
            .field("message")
            .indexed_field("repeated", 3)
            .field("nested");

        assert_eq!(
            complex_location.path_string(),
            ".message.repeated[3].nested"
        );
    }

    #[test]
    fn test_proto_location_display() {
        // Using a simple struct for testing
        #[derive(Debug)]
        struct TestProto {}

        let test_proto = TestProto {};
        let location = ProtoLocation::new(&test_proto);

        // Base location (no path)
        let display_str = format!("{}", location);
        assert!(display_str.starts_with("proto type"));
        assert!(display_str.contains(&format!("0x{:x}", &test_proto as *const TestProto as u64)));

        // Location with path
        let location_with_path = location.field("test_field");
        let display_str = format!("{}", location_with_path);
        assert!(display_str.ends_with(".test_field"));

        // Unknown location
        let unknown = ProtoLocation::unknown();
        assert_eq!(format!("{}", unknown), "unknown proto location");
    }

    #[test]
    fn test_proto_location_equality() {
        // Using two simple structs for testing
        #[derive(Debug)]
        struct TestProto1 {}

        #[derive(Debug)]
        struct TestProto2 {}

        let test_proto1 = TestProto1 {};
        let same_proto1 = TestProto1 {};
        let test_proto2 = TestProto2 {};

        let location1 = ProtoLocation::new(&test_proto1);
        let another_location1 = ProtoLocation::new(&same_proto1);
        let location2 = ProtoLocation::new(&test_proto2);

        // Locations from same type but different instances should be different
        assert_ne!(location1, another_location1);

        // Locations from different types should be different
        assert_ne!(location1, location2);

        // Cloned location should be equal to original
        let cloned_location = location1.clone();
        assert_eq!(location1, cloned_location);

        // Locations with different paths should be different
        let location_with_field = location1.field("test_field");
        assert_ne!(location1, location_with_field);
    }

    #[test]
    fn test_location_trait_methods() {
        // Using a simple struct for testing
        #[derive(Debug)]
        struct TestProto {}

        let test_proto = TestProto {};
        let location = ProtoLocation::new(&test_proto);

        // Test is_unknown
        assert!(!location.is_unknown());
        assert!(ProtoLocation::unknown().is_unknown());

        // Test location_hash
        let hash1 = location.location_hash();
        let hash2 = location.field("test").location_hash();
        assert_ne!(hash1, hash2);

        // Test equals with same type
        let cloned = location.clone();
        assert!(location.equals(&cloned));
        assert!(!location.equals(&location.field("test")));

        // Test equals with different location type
        let unknown_loc = UnknownLocation::UNKNOWN;
        assert!(!location.equals(&unknown_loc));

        // Test box_clone
        let boxed = location.box_clone();
        assert!(location.equals(&*boxed));
    }

    #[test]
    fn test_hash_consistency() {
        // Using a simple struct for testing
        #[derive(Debug)]
        struct TestProto {}

        let test_proto = TestProto {};
        let location = ProtoLocation::new(&test_proto);

        // Test that Hash impl is consistent with location_hash
        let mut hasher1 = DefaultHasher::new();
        location.hash(&mut hasher1);
        let std_hash = hasher1.finish();

        // The hashes won't be identical because the implementations are different,
        // but we can check that modifications affect both consistently
        let location2 = location.field("test");

        let mut hasher2 = DefaultHasher::new();
        location2.hash(&mut hasher2);
        let std_hash2 = hasher2.finish();

        // Both hashing methods should recognize these as different locations
        assert_ne!(location.location_hash(), location2.location_hash());
        assert_ne!(std_hash, std_hash2);
    }

    #[test]
    fn test_nested_message_traversal() {
        // Define a nested protobuf-like structure
        #[derive(Debug)]
        struct NestedProtoField {
            value: i32,
            name: String,
        }

        #[derive(Debug)]
        struct NestedProto {
            id: i32,
            nested_field: NestedProtoField,
            repeated_fields: Vec<NestedProtoField>,
        }

        // Create test instance
        let nested_field = NestedProtoField {
            value: 42,
            name: "inner".to_string(),
        };

        let repeated_fields = vec![
            NestedProtoField {
                value: 1,
                name: "first".to_string(),
            },
            NestedProtoField {
                value: 2,
                name: "second".to_string(),
            },
        ];

        let proto = NestedProto {
            id: 123,
            nested_field,
            repeated_fields,
        };

        // Starting location at the root message
        let root_location = ProtoLocation::new(&proto);

        // Test traversal to nested message
        let nested_location = root_location.field("nested_field");
        assert_eq!(nested_location.path_string(), ".nested_field");

        // Test traversal to field within nested message
        let value_location = nested_location.field("value");
        assert_eq!(value_location.path_string(), ".nested_field.value");

        // Test traversal to repeated field
        let repeated_location = root_location.field("repeated_fields");
        assert_eq!(repeated_location.path_string(), ".repeated_fields");

        // Test traversal to specific element in repeated field
        let first_element_location = repeated_location.indexed_field("element", 0);
        assert_eq!(
            first_element_location.path_string(),
            ".repeated_fields.element[0]"
        );

        // Test traversal to field within a repeated element
        let element_name_location = first_element_location.field("name");
        assert_eq!(
            element_name_location.path_string(),
            ".repeated_fields.element[0].name"
        );

        // Test alternative direct path construction
        let direct_path = root_location
            .field("repeated_fields")
            .indexed_field("element", 1)
            .field("value");
        assert_eq!(
            direct_path.path_string(),
            ".repeated_fields.element[1].value"
        );
    }

    #[test]
    fn test_visitor_pattern_usage() {
        // Define a simple visitor pattern simulation

        // Message types
        #[derive(Debug)]
        struct Plan {
            relations: Vec<Relation>,
            version: String,
        }

        #[derive(Debug)]
        struct Relation {
            name: String,
            expressions: Vec<Expression>,
        }

        #[derive(Debug)]
        struct Expression {
            expr_type: String,
            value: String,
        }

        // Create test data
        let expr1 = Expression {
            expr_type: "literal".to_string(),
            value: "42".to_string(),
        };

        let expr2 = Expression {
            expr_type: "reference".to_string(),
            value: "column_a".to_string(),
        };

        let relation = Relation {
            name: "test_relation".to_string(),
            expressions: vec![expr1, expr2],
        };

        let plan = Plan {
            relations: vec![relation],
            version: "1.0".to_string(),
        };

        // Simulate visitor traversal with location tracking
        let plan_location = ProtoLocation::new(&plan);

        // Visit plan fields
        let version_location = plan_location.field("version");
        assert_eq!(version_location.path_string(), ".version");

        // Visit relations
        let relations_location = plan_location.field("relations");
        assert_eq!(relations_location.path_string(), ".relations");

        // Visit first relation
        let relation0_location = relations_location.indexed_field("rel", 0);
        assert_eq!(relation0_location.path_string(), ".relations.rel[0]");

        // Visit relation name
        let relation_name_location = relation0_location.field("name");
        assert_eq!(
            relation_name_location.path_string(),
            ".relations.rel[0].name"
        );

        // Visit expressions
        let expressions_location = relation0_location.field("expressions");
        assert_eq!(
            expressions_location.path_string(),
            ".relations.rel[0].expressions"
        );

        // Visit specific expression and its fields
        let expr0_location = expressions_location.indexed_field("expr", 0);
        let expr0_type_location = expr0_location.field("expr_type");
        let expr0_value_location = expr0_location.field("value");

        assert_eq!(
            expr0_location.path_string(),
            ".relations.rel[0].expressions.expr[0]"
        );
        assert_eq!(
            expr0_type_location.path_string(),
            ".relations.rel[0].expressions.expr[0].expr_type"
        );
        assert_eq!(
            expr0_value_location.path_string(),
            ".relations.rel[0].expressions.expr[0].value"
        );

        // Visit another expression directly
        let expr1_value_location = plan_location
            .field("relations")
            .indexed_field("rel", 0)
            .field("expressions")
            .indexed_field("expr", 1)
            .field("value");

        assert_eq!(
            expr1_value_location.path_string(),
            ".relations.rel[0].expressions.expr[1].value"
        );

        // Verify different paths have different hashes
        assert_ne!(
            expr0_value_location.location_hash(),
            expr1_value_location.location_hash()
        );
    }

    #[test]
    fn test_proto_location_macro() {
        // Test the proto_location! macro
        #[derive(Debug)]
        struct TestMessage {
            id: i32,
        }

        let message = TestMessage { id: 100 };

        // Use the macro
        let location = proto_location!(message);

        // Verify the location
        assert!(!location.is_unknown());
        assert_eq!(location.root_type_id(), TypeId::of::<TestMessage>());
        assert_eq!(
            location.root_instance_id(),
            &message as *const TestMessage as u64
        );
        assert!(location.path().is_empty());

        // Test with chained methods
        let field_location = proto_location!(message).field("id");
        assert_eq!(field_location.path_string(), ".id");
    }

    #[test]
    #[should_panic(expected = "Field name cannot be empty for indexed field")]
    fn test_empty_field_name_panics() {
        // Using a simple struct for testing
        #[derive(Debug)]
        struct TestProto {}

        let test_proto = TestProto {};
        let location = ProtoLocation::new(&test_proto);

        // This should panic because empty field names are not allowed
        let _ = location.indexed_field("", 0);
    }
}
