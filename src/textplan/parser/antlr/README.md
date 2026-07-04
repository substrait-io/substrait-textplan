# ANTLR4 Parser for Substrait TextPlan

This directory will contain the ANTLR4-generated code for parsing the Substrait TextPlan format.

## Code Generation

The ANTLR code is generated from the grammar files located in `src/substrait/textplan/parser/grammar/`.

To generate the parser code, you need:

1. Java installed on your system
2. The special ANTLR4 tool with Rust support

### Special ANTLR4 JAR with Rust Support

**IMPORTANT:** You must use the special ANTLR4 JAR file that supports generating Rust code. The standard ANTLR4 JAR files do NOT support Rust.

Download the JAR file with Rust support:

```bash
# Download special ANTLR4 JAR file with Rust support (one-time setup)
curl -L -o antlr4rust.jar https://github.com/rrevenantt/antlr4rust/releases/download/antlr4-4.8-2-Rust-0.3.0-beta/antlr4-4.8-2-SNAPSHOT-complete.jar
```

### Generate the Code

Set the environment variable to point to the JAR and generate the code:

```bash
# Set the JAR location as an environment variable
export ANTLR_JAR=/path/to/antlr4rust.jar

# Generate the code
GENERATE_ANTLR=true cargo build
```

## Generated Code Structure

The ANTLR code generation will create several Rust files:

- `substrait_plan_lexer.rs`: Lexer implementation
- `substrait_plan_parser.rs`: Parser implementation
- `substrait_plan_visitor.rs`: Visitor trait
- `substrait_plan_base_visitor.rs`: Base visitor implementation

## Integration with Existing Code

The parser implementation follows the same multiphase approach as the C++ implementation:

1. First phase: Basic parsing and symbol table creation (TypeVisitor + PlanVisitor)
2. Second phase: Pipeline processing (PipelineVisitor)
3. Third phase: Relation and expression processing (RelationVisitor)
4. Fourth phase: Subquery processing (SubqueryRelationVisitor)

Each phase is implemented with a separate visitor that builds on the results of the previous phase.

### Visitor Implementation Pattern

To implement an ANTLR visitor in Rust, you'll need to:

1. Create a struct for your visitor that will hold any needed state
2. Implement the `SubstraitPlanVisitor` trait for your visitor
3. Override the `visit_*` methods for the grammar rules you want to process

Example:

```rust
pub struct PlanVisitor {
    symbol_table: SymbolTable,
    error_listener: Arc<ErrorListener>,
}

impl PlanVisitor {
    pub fn new(symbol_table: SymbolTable, error_listener: Arc<ErrorListener>) -> Self {
        Self { symbol_table, error_listener }
    }
}

impl<'input> SubstraitPlanVisitor<'input> for PlanVisitor {
    fn visit_plan(&mut self, ctx: &PlanContext<'input>) -> /* return type */ {
        // Process the plan node
    }
    
    // Implement other visit_* methods as needed
}
```

## Transition from Tree-Sitter

During the transition period, both Tree-Sitter and ANTLR parsers are available. The code is configured 
to use the ANTLR parser when available, with Tree-Sitter as a fallback.

## Complete Implementation Steps

To complete the ANTLR parser implementation:

1. Generate the ANTLR code as described above
2. Implement the visitors that correspond to the C++ implementation:
   - `TypeVisitor`: Handles type information and conversions
   - `PlanVisitor`: Builds the initial symbol table
   - `PipelineVisitor`: Processes pipeline definitions
   - `RelationVisitor`: Processes relations and expressions
   - `SubqueryRelationVisitor`: Handles subquery processing
3. Update the parse_text.rs implementation to use the ANTLR-generated parser
4. Add tests that verify the ANTLR parser works correctly
5. Once everything is working, remove the Tree-Sitter implementation
