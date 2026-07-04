# Substrait TextPlan Converter

This module provides functionality to convert between different representations of Substrait plans:

- Binary protobuf format ⟷ JSON format
- Binary protobuf format ⟷ text plan format
- JSON format ⟷ text plan format

## Visitor Pattern

The converter uses the visitor pattern to traverse and process Substrait plans. The `BasePlanProtoVisitor` trait defines a set of methods for visiting all parts of a Substrait plan, and concrete implementations can override specific methods to implement custom behavior.

### Visitor Generator

The included visitor generator tool automatically generates a `BasePlanProtoVisitor` implementation from Substrait protobuf schema. This ensures that the visitor covers all possible message types in the Substrait protocol.

To use the generator:

```bash
cargo run --bin generate_visitor -- third_party/substrait/proto src/textplan/converter/generated_visitor.rs
```

This will parse the Substrait protobuf files in `third_party/substrait/proto` and generate a complete visitor implementation in `src/textplan/converter/generated_visitor.rs`.

### Creating Custom Visitors

To create a custom visitor, implement the `BasePlanProtoVisitor` trait or extend the `DefaultPlanVisitor` struct. You can override specific methods to process only the parts of the plan you're interested in.

Example:

```rust
use crate::textplan::converter::visitor::{BasePlanProtoVisitor, DefaultPlanVisitor};
use crate::proto::substrait;

struct MyCustomVisitor {
    // Custom state for your visitor
    relation_count: usize,
}

impl MyCustomVisitor {
    fn new() -> Self {
        Self {
            relation_count: 0,
        }
    }
}

impl BasePlanProtoVisitor for MyCustomVisitor {
    type Result = Result<(), TextPlanError>;

    fn visit_plan(&mut self, plan: &substrait::Plan) -> Self::Result {
        // Custom implementation for visiting a plan
        println!("Visiting plan with {} relations", plan.relations.len());
        
        // Call the default implementation to visit all parts of the plan
        for relation in &plan.relations {
            if let Some(rel_type) = &relation.rel_type {
                match rel_type {
                    substrait::plan_rel::RelType::Rel(rel) => {
                        self.visit_relation(rel)?;
                    }
                    substrait::plan_rel::RelType::Root(_) => {
                        // Handle root relation
                    }
                }
            }
        }
        
        Ok(())
    }

    fn visit_relation(&mut self, relation: &substrait::Rel) -> Self::Result {
        // Count relations
        self.relation_count += 1;
        
        // Continue traversal
        if let Some(rel_type) = &relation.rel_type {
            match rel_type {
                substrait::rel::RelType::Read(read_rel) => {
                    self.visit_read_relation(read_rel)?;
                }
                // Handle other relation types
                _ => {}
            }
        }
        
        Ok(())
    }
}
```

## Converter API

The converter module provides the following main functions:

- `load_from_binary`: Load a Substrait plan from binary protobuf format
- `save_to_binary`: Save a Substrait plan to binary protobuf format
- `load_from_json`: Load a Substrait plan from JSON format
- `save_to_json`: Save a Substrait plan to JSON format

These functions are re-exported at the module level for convenience.

## Future Work

- Complete implementation of text plan to binary conversion
- Add more comprehensive unit tests
- Improve error handling and reporting
- Add support for extended expressions