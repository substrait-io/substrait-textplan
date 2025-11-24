// SPDX-License-Identifier: Apache-2.0

//! Grammar module for the ANTLR4 parser.

use crate::textplan::parser::antlr::substraitplanparser::SubstraitPlanParserContextType;
use crate::textplan::parser::antlr::{SubstraitPlanLexer, SubstraitPlanParser};
use crate::textplan::parser::error_listener::ErrorListener;
use crate::textplan::TextLocation;
use antlr_rust::{
    common_token_stream::CommonTokenStream, input_stream::InputStream, parser::Parser,
    token_factory::CommonTokenFactory, DefaultErrorStrategy,
};
use std::fs;
use std::path::Path;
use std::sync::Arc;

/// Options for the ANTLR parser.
pub struct AntlrParserOptions {
    /// The directory containing the grammar files.
    pub grammar_dir: String,
}

impl Default for AntlrParserOptions {
    fn default() -> Self {
        Self {
            grammar_dir: "src/substrait/textplan/parser/grammar".to_string(),
        }
    }
}

// Re-export PlanContext for simplicity
pub use crate::textplan::parser::antlr::substraitplanparser::PlanContext;
use crate::textplan::parser::error_listener::create_boxed_error_listener;
use crate::textplan::parser::error_listener::AntlrErrorListener;

/// Result from parsing a string using ANTLR.
/// This now contains the processed symbol table instead of the raw parse tree.
pub struct ParseResult {
    /// The symbol table built by processing the parse tree.
    pub symbol_table: crate::textplan::symbol_table::SymbolTable,
    /// The error listener that captured any syntax errors.
    pub error_listener: Arc<ErrorListener>,
}

/// Parses a string using ANTLR.
///
/// Creates a lexer, parser, and runs the parsing process on the input text.
/// Processes the parse tree with visitors while the lexer/parser are still alive.
///
/// # Arguments
///
/// * `text` - The text to parse.
///
/// # Returns
///
/// A result containing either a processed symbol table or an error message.
pub fn parse_string(text: &str) -> Result<ParseResult, String> {
    use crate::textplan::parser::visitors::PlanVisitor as PlanVisitorTrait;
    use crate::textplan::symbol_table::SymbolTable;

    // Create an error listener
    let error_listener = Arc::new(ErrorListener::new());

    // Create a token factory - this needs to outlive the lexer
    let tf = CommonTokenFactory;

    // Create a lexer
    let lexer_result = create_lexer(text, error_listener.clone(), &tf);

    // If lexer creation fails, return the error
    let lexer = match lexer_result {
        Ok(mut lexer) => {
            // Create an ANTLR error listener
            let boxed_error_listener = create_boxed_error_listener(error_listener.clone());

            // Add the error listener to the lexer
            lexer.remove_error_listeners();
            lexer.add_error_listener(boxed_error_listener);

            lexer
        }
        Err(err) => return Err(err),
    };

    // Create a token stream from the lexer
    let token_stream = CommonTokenStream::new(lexer);

    // Create a parser with the token stream and default error strategy
    let error_strategy = DefaultErrorStrategy::new();
    let mut parser = SubstraitPlanParser::with_strategy(token_stream, error_strategy);

    // Create an ANTLR error listener for the parser
    let boxed_error_listener = create_boxed_error_listener(error_listener.clone());

    // Add the error listener to the parser
    parser.remove_error_listeners();
    parser.add_error_listener(boxed_error_listener);

    // Call the parser's plan() method to get the parse tree
    let plan_result = match parser.plan() {
        Ok(plan_ctx) => plan_ctx,
        Err(e) => {
            // If we get a parsing error, add it to our error listener
            error_listener.add_error(
                format!("Parser error: {}", e),
                TextLocation::new(0, 0), // We don't have specific location for this error
            );
            // Return early with the error
            return Err(format!("Parser error: {}", e));
        }
    };

    // Check for errors collected by our custom error listener
    if error_listener.has_errors() {
        let error_messages = error_listener.format_errors();
        return Err(format!("Parsing errors: {}", error_messages.join(", ")));
    }

    // Process the parse tree with visitors while the lexer/parser are still alive.
    // This is critical because the parse tree has references to the token stream.
    let mut symbol_table = SymbolTable::new();

    // Process the parse tree in multiple phases using our visitors
    // Phase 1: Type visitor
    println!("Applying TypeVisitor");
    let mut type_visitor =
        crate::textplan::parser::visitors::TypeVisitor::new(symbol_table, error_listener.clone());
    crate::textplan::parser::visitors::visit_plan(&mut type_visitor, plan_result.as_ref());
    symbol_table = type_visitor.symbol_table();

    // Phase 2: Main plan visitor
    println!("Applying MainPlanVisitor");
    let mut plan_visitor = crate::textplan::parser::visitors::MainPlanVisitor::new(
        symbol_table,
        error_listener.clone(),
    );
    crate::textplan::parser::visitors::visit_plan(&mut plan_visitor, plan_result.as_ref());
    symbol_table = plan_visitor.symbol_table();

    // Phase 3: Pipeline visitor
    println!("Applying PipelineVisitor");
    let mut pipeline_visitor = crate::textplan::parser::visitors::PipelineVisitor::new(
        symbol_table,
        error_listener.clone(),
    );
    crate::textplan::parser::visitors::visit_plan(&mut pipeline_visitor, plan_result.as_ref());
    symbol_table = pipeline_visitor.symbol_table();

    // Phase 4: Relation visitor
    println!("Applying RelationVisitor");
    let mut relation_visitor = crate::textplan::parser::visitors::RelationVisitor::new(
        symbol_table,
        error_listener.clone(),
    );

    crate::textplan::parser::visitors::visit_plan(&mut relation_visitor, plan_result.as_ref());
    symbol_table = relation_visitor.symbol_table();

    // Phase 5: Subquery relation visitor
    println!("Applying SubqueryRelationVisitor");
    let mut subquery_visitor = crate::textplan::parser::visitors::SubqueryRelationVisitor::new(
        symbol_table,
        error_listener.clone(),
    );
    crate::textplan::parser::visitors::visit_plan(&mut subquery_visitor, plan_result.as_ref());

    // Populate sub_query_pipelines now that parent_query_index has been set during the visit
    subquery_visitor.finalize();

    symbol_table = subquery_visitor.symbol_table();

    // Outer references were already fixed by SubqueryRelationVisitor during its visit pass.
    // No need for additional fixing here.

    // Return the parse result with the processed symbol table
    Ok(ParseResult {
        symbol_table,
        error_listener,
    })
}

/// Fixes field references in subquery relations to use outerReference instead of rootReference
/// when they refer to parent query fields.
fn fix_outer_references_in_subqueries(
    symbol_table: &mut crate::textplan::symbol_table::SymbolTable,
) {
    use crate::textplan::common::structured_symbol_data::RelationData;
    use crate::textplan::symbol_table::SymbolType;
    use std::sync::Arc;

    // Find all subquery relations (those with parent_query_index >= 0)
    let subquery_relations: Vec<Arc<crate::textplan::symbol_table::SymbolInfo>> = symbol_table
        .symbols()
        .iter()
        .filter(|s| s.symbol_type() == SymbolType::Relation && s.parent_query_index() >= 0)
        .cloned()
        .collect();

    println!(
        "  Found {} subquery relations to process",
        subquery_relations.len()
    );

    for subquery_rel in &subquery_relations {
        println!("  Processing subquery relation '{}'", subquery_rel.name());

        // Fix outer references in this relation's expressions
        if let Some(blob_lock) = &subquery_rel.blob {
            if let Ok(mut blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_mut::<RelationData>() {
                    let field_count = relation_data.field_references.len();
                    fix_outer_refs_in_rel(&mut relation_data.relation, field_count, symbol_table);
                }
            }
        }
    }
}

/// Recursively fixes outer references in a relation and its nested expressions.
fn fix_outer_refs_in_rel(
    rel: &mut substrait::proto::Rel,
    subquery_field_count: usize,
    symbol_table: &crate::textplan::symbol_table::SymbolTable,
) {
    use substrait::proto::rel::RelType;

    match &mut rel.rel_type {
        Some(RelType::Filter(filter)) => {
            if let Some(condition) = &mut filter.condition {
                fix_outer_refs_in_expression(condition, subquery_field_count, symbol_table);
            }
        }
        Some(RelType::Project(project)) => {
            for expr in &mut project.expressions {
                fix_outer_refs_in_expression(expr, subquery_field_count, symbol_table);
            }
        }
        Some(RelType::Join(join)) => {
            if let Some(expr) = &mut join.expression {
                fix_outer_refs_in_expression(expr, subquery_field_count, symbol_table);
            }
        }
        _ => {}
    }
}

/// Recursively fixes outer references in an expression.
fn fix_outer_refs_in_expression(
    expr: &mut substrait::proto::Expression,
    subquery_field_count: usize,
    symbol_table: &crate::textplan::symbol_table::SymbolTable,
) {
    use substrait::proto::expression::field_reference::ReferenceType;
    use substrait::proto::expression::field_reference::RootType;
    use substrait::proto::expression::RexType;

    match &mut expr.rex_type {
        Some(RexType::Selection(selection)) => {
            // Check if this is a rootReference that should be an outerReference
            if matches!(&selection.root_type, Some(RootType::RootReference(_))) {
                // This is a root reference - check if it should be outer
                if let Some(ref_type) = &selection.reference_type {
                    if let ReferenceType::DirectReference(direct_ref) = ref_type {
                        if let Some(substrait::proto::expression::reference_segment::ReferenceType::StructField(struct_field)) = &direct_ref.reference_type {
                            // We need to determine if this field belongs to the current relation's schema
                            // or to the parent relation's schema

                            // For now, check based on field index compared to schema sizes
                            // This is a simplified heuristic - ideally we'd track schema associations
                            let field_index = struct_field.field as usize;

                            // If field_index >= subquery_field_count, it's likely an outer reference
                            // (subquery_field_count was passed as a parameter to avoid re-locking)
                            println!("      Checking field[{}] against subquery field_count={}", field_index, subquery_field_count);
                            if field_index >= subquery_field_count && subquery_field_count > 0 {
                                println!("      Converting field[{}] from rootReference to outerReference (subquery has {} fields)",
                                    field_index, subquery_field_count);

                                // Convert to outer reference
                                selection.root_type = Some(RootType::OuterReference(
                                    substrait::proto::expression::field_reference::OuterReference {
                                        steps_out: 1,
                                    }
                                ));
                            }
                        }
                    }
                }
            }
        }
        Some(RexType::ScalarFunction(func)) => {
            for arg in &mut func.arguments {
                if let Some(substrait::proto::function_argument::ArgType::Value(inner_expr)) =
                    &mut arg.arg_type
                {
                    fix_outer_refs_in_expression(inner_expr, subquery_field_count, symbol_table);
                }
            }
        }
        Some(RexType::Cast(cast)) => {
            if let Some(input) = &mut cast.input {
                fix_outer_refs_in_expression(input, subquery_field_count, symbol_table);
            }
        }
        Some(RexType::Subquery(subquery)) => {
            use substrait::proto::expression::subquery::SubqueryType;

            // Don't recurse into nested subqueries - they'll be handled separately
            if let Some(SubqueryType::SetComparison(set_comp)) = &mut subquery.subquery_type {
                if let Some(left) = &mut set_comp.left {
                    fix_outer_refs_in_expression(left, subquery_field_count, symbol_table);
                }
            }
        }
        _ => {}
    }
}

/// Creates a new lexer for the given input text.
///
/// # Arguments
///
/// * `text` - The text to tokenize.
/// * `error_listener` - The error listener to use.
/// * `tf` - The token factory to use for the lexer.
///
/// # Returns
///
/// A result containing either a lexer or an error message.
pub fn create_lexer<'input, 'tf>(
    text: &'input str,
    error_listener: Arc<ErrorListener>,
    tf: &'tf CommonTokenFactory,
) -> Result<SubstraitPlanLexer<'input, InputStream<&'input str>>, String>
where
    'tf: 'input, // This constraint ensures that tf lives at least as long as input
{
    let input = InputStream::new(text);
    let mut lexer = SubstraitPlanLexer::new_with_token_factory(input, tf);

    // Create an ANTLR error listener that will forward errors to our ErrorListener
    let antlr_error_listener = AntlrErrorListener::new(error_listener.clone());

    // Add the error listener to the lexer
    lexer.add_error_listener(Box::new(antlr_error_listener));

    Ok(lexer)
}

/// Creates a new parser for the given input text.
///
/// # Arguments
///
/// * `token_stream` - The token stream to parse
/// * `error_listener` - The error listener to use.
///
/// # Returns
///
/// A result containing either a parser or an error message.
pub fn create_parser<'input>(
    token_stream: CommonTokenStream<'input, SubstraitPlanLexer<'input, InputStream<&'input str>>>,
    error_listener: Arc<ErrorListener>,
) -> Result<
    SubstraitPlanParser<
        'input,
        CommonTokenStream<'input, SubstraitPlanLexer<'input, InputStream<&'input str>>>,
        DefaultErrorStrategy<'input, SubstraitPlanParserContextType>,
    >,
    String,
> {
    // Create a parser with the token stream and default error strategy
    let error_strategy = DefaultErrorStrategy::new();
    let mut parser = SubstraitPlanParser::with_strategy(token_stream, error_strategy);

    // Create an ANTLR error listener for the parser
    let boxed_error_listener = create_boxed_error_listener(error_listener);

    // Add the error listener to the parser
    parser.remove_error_listeners();
    parser.add_error_listener(boxed_error_listener);

    Ok(parser)
}

/// Reads grammar files from the specified directory.
///
/// This function is useful for diagnostics and debugging.
///
/// # Arguments
///
/// * `options` - Options for the parser, including the grammar directory.
///
/// # Returns
///
/// A result containing either a tuple of (lexer_grammar, parser_grammar) or an error message.
pub fn read_grammar_files(options: &AntlrParserOptions) -> Result<(String, String), String> {
    let grammar_dir = Path::new(&options.grammar_dir);

    // Read lexer grammar
    let lexer_path = grammar_dir.join("SubstraitPlanLexer.g4");
    let lexer_grammar = fs::read_to_string(&lexer_path)
        .map_err(|e| format!("Failed to read lexer grammar: {}", e))?;

    // Read parser grammar
    let parser_path = grammar_dir.join("SubstraitPlanParser.g4");
    let parser_grammar = fs::read_to_string(&parser_path)
        .map_err(|e| format!("Failed to read parser grammar: {}", e))?;

    Ok((lexer_grammar, parser_grammar))
}

/// Checks if a parse tree has any syntax errors.
///
/// # Arguments
///
/// * `error_listener` - The error listener to check.
///
/// # Returns
///
/// `true` if the error listener has any errors, `false` otherwise.
pub fn has_errors(error_listener: &ErrorListener) -> bool {
    error_listener.has_errors()
}
