// SPDX-License-Identifier: Apache-2.0

//! Expression printer for converting Substrait proto expressions to text format.

use std::sync::Arc;

use crate::textplan::common::error::TextPlanError;
use crate::textplan::common::structured_symbol_data::{FunctionData, RelationData};
use crate::textplan::symbol_table::{SymbolInfo, SymbolTable, SymbolType};

/// A printer for converting Substrait proto expressions to textplan format.
///
/// The ExpressionPrinter handles conversion of Expression protobuf messages
/// into human-readable text representations, including literals, field references,
/// scalar functions, and type annotations.
pub struct ExpressionPrinter<'a> {
    /// The symbol table for looking up function names
    symbol_table: &'a SymbolTable,
    /// The current scope (relation) for resolving field references
    current_scope: Option<&'a Arc<SymbolInfo>>,
    /// Depth of nested function calls (for potential future formatting)
    function_depth: usize,
    /// Index for tracking subquery lookups within the current scope
    current_scope_index: i32,
}

impl<'a> ExpressionPrinter<'a> {
    /// Creates a new ExpressionPrinter with the given symbol table and scope.
    pub fn new(symbol_table: &'a SymbolTable, current_scope: Option<&'a Arc<SymbolInfo>>) -> Self {
        Self {
            symbol_table,
            current_scope,
            function_depth: 0,
            current_scope_index: 0,
        }
    }

    /// Prints an expression to a string.
    pub fn print_expression(
        &mut self,
        expr: &::substrait::proto::Expression,
    ) -> Result<String, TextPlanError> {
        use ::substrait::proto::expression::RexType;

        match &expr.rex_type {
            Some(RexType::Literal(lit)) => self.print_literal(lit),
            Some(RexType::Selection(sel)) => self.print_field_reference(sel),
            Some(RexType::ScalarFunction(func)) => self.print_scalar_function(func),
            Some(RexType::WindowFunction(_)) => {
                Ok("WINDOW_FUNCTION_NOT_YET_IMPLEMENTED".to_string())
            }
            Some(RexType::IfThen(if_then)) => self.print_if_then(if_then),
            Some(RexType::SwitchExpression(_)) => {
                Ok("SWITCH_EXPRESSION_NOT_YET_IMPLEMENTED".to_string())
            }
            Some(RexType::SingularOrList(_)) => {
                Ok("SINGULAR_OR_LIST_NOT_YET_IMPLEMENTED".to_string())
            }
            Some(RexType::MultiOrList(_)) => Ok("MULTI_OR_LIST_NOT_YET_IMPLEMENTED".to_string()),
            Some(RexType::Cast(cast)) => self.print_cast(cast),
            Some(RexType::Subquery(subquery)) => self.print_subquery(subquery),
            Some(RexType::Nested(_)) => Ok("NESTED_NOT_YET_IMPLEMENTED".to_string()),
            Some(RexType::Enum(_)) => Ok("ENUM_NOT_YET_IMPLEMENTED".to_string()),
            Some(RexType::DynamicParameter(_)) => {
                Ok("DYNAMIC_PARAMETER_NOT_YET_IMPLEMENTED".to_string())
            }
            None => Err(TextPlanError::InvalidExpression(
                "Expression has no rex_type".to_string(),
            )),
        }
    }

    /// Prints a literal expression.
    fn print_literal(
        &self,
        literal: &::substrait::proto::expression::Literal,
    ) -> Result<String, TextPlanError> {
        use ::substrait::proto::expression::literal::LiteralType;

        let mut result = match &literal.literal_type {
            Some(LiteralType::Boolean(b)) => b.to_string(),
            Some(LiteralType::I8(v)) => format!("{}_i8", v),
            Some(LiteralType::I16(v)) => format!("{}_i16", v),
            Some(LiteralType::I32(v)) => format!("{}_i32", v),
            Some(LiteralType::I64(v)) => format!("{}_i64", v),
            Some(LiteralType::Fp32(v)) => format!("{}_fp32", v),
            Some(LiteralType::Fp64(v)) => format!("{}_fp64", v),
            Some(LiteralType::String(s)) => format!("\"{}\"", escape_string(s)),
            Some(LiteralType::Binary(_)) => "BINARY_LITERAL_NOT_YET_IMPLEMENTED".to_string(),
            Some(LiteralType::Timestamp(_)) => "TIMESTAMP_LITERAL_NOT_YET_IMPLEMENTED".to_string(),
            Some(LiteralType::Date(days)) => format!("{}_date", days),
            Some(LiteralType::Time(micros)) => format!("{}_time", micros),
            Some(LiteralType::IntervalYearToMonth(interval)) => {
                // IntervalYearToMonth has years and months fields
                // Use struct literal format for multiple components
                if interval.months != 0 {
                    format!(
                        "{{{}, {}}}_interval_year_month",
                        interval.years, interval.months
                    )
                } else {
                    format!("{}_interval_year", interval.years)
                }
            }
            Some(LiteralType::IntervalDayToSecond(interval)) => {
                // IntervalDayToSecond: Use deprecated microseconds format for compatibility
                // The deprecated format stores microseconds in precision_mode.Microseconds variant
                use ::substrait::proto::expression::literal::interval_day_to_second::PrecisionMode;

                let microseconds = match &interval.precision_mode {
                    Some(PrecisionMode::Microseconds(micros)) => *micros,
                    Some(PrecisionMode::Precision(_)) | None => {
                        // Convert subseconds to microseconds (assuming precision 6)
                        interval.subseconds as i32
                    }
                };

                format!(
                    "{{{}, {}, {}}}_interval_day_second",
                    interval.days, interval.seconds, microseconds
                )
            }
            Some(LiteralType::FixedChar(s)) => format!("\"{}\"_fixedchar", escape_string(s)),
            Some(LiteralType::VarChar(val)) => {
                // VarChar literal has value and length fields
                if !val.value.is_empty() {
                    format!("\"{}\"_varchar", escape_string(&val.value))
                } else if val.length > 0 {
                    format!("\"\"_varchar<{}>", val.length)
                } else {
                    "\"\"_varchar".to_string()
                }
            }
            Some(LiteralType::FixedBinary(bytes)) => {
                // Convert bytes to hex string manually
                let hex_str: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
                format!("0x{}_fixedbinary", hex_str)
            }
            Some(LiteralType::Decimal(dec)) => {
                // Decimal is stored as 16 bytes (little-endian two's complement)
                // Convert to i128 and then to string
                let mut bytes = [0u8; 16];
                if dec.value.len() >= 16 {
                    bytes.copy_from_slice(&dec.value[0..16]);
                } else {
                    bytes[..dec.value.len()].copy_from_slice(&dec.value);
                }
                let value = i128::from_le_bytes(bytes);
                format!("{}_decimal<{},{}>", value, dec.precision, dec.scale)
            }
            Some(LiteralType::Struct(_)) => "STRUCT_LITERAL_NOT_YET_IMPLEMENTED".to_string(),
            Some(LiteralType::Map(_)) => "MAP_LITERAL_NOT_YET_IMPLEMENTED".to_string(),
            Some(LiteralType::TimestampTz(_)) => {
                "TIMESTAMP_TZ_LITERAL_NOT_YET_IMPLEMENTED".to_string()
            }
            Some(LiteralType::Uuid(_)) => "UUID_LITERAL_NOT_YET_IMPLEMENTED".to_string(),
            Some(LiteralType::Null(_)) => "NULL".to_string(),
            Some(LiteralType::List(_)) => "LIST_LITERAL_NOT_YET_IMPLEMENTED".to_string(),
            Some(LiteralType::EmptyList(_)) => "EMPTY_LIST_LITERAL_NOT_YET_IMPLEMENTED".to_string(),
            Some(LiteralType::EmptyMap(_)) => "EMPTY_MAP_LITERAL_NOT_YET_IMPLEMENTED".to_string(),
            Some(LiteralType::UserDefined(_)) => {
                "USER_DEFINED_LITERAL_NOT_YET_IMPLEMENTED".to_string()
            }
            Some(LiteralType::IntervalCompound(_)) => {
                "INTERVAL_COMPOUND_LITERAL_NOT_YET_IMPLEMENTED".to_string()
            }
            Some(LiteralType::PrecisionTime(_)) => {
                "PRECISION_TIME_LITERAL_NOT_YET_IMPLEMENTED".to_string()
            }
            Some(LiteralType::PrecisionTimestamp(_)) => {
                "PRECISION_TIMESTAMP_LITERAL_NOT_YET_IMPLEMENTED".to_string()
            }
            Some(LiteralType::PrecisionTimestampTz(_)) => {
                "PRECISION_TIMESTAMP_TZ_LITERAL_NOT_YET_IMPLEMENTED".to_string()
            }
            None => {
                return Err(TextPlanError::InvalidExpression(
                    "Literal has no literal_type".to_string(),
                ))
            }
        };

        // Add nullable marker if needed
        if literal.nullable {
            result.push('?');
        }

        Ok(result)
    }

    /// Prints a field reference (selection).
    fn print_field_reference(
        &self,
        field_ref: &::substrait::proto::expression::FieldReference,
    ) -> Result<String, TextPlanError> {
        use ::substrait::proto::expression::field_reference::ReferenceType;

        match &field_ref.reference_type {
            Some(ReferenceType::DirectReference(direct_ref)) => {
                // Extract outer reference if it exists from root_type
                let outer_ref = match &field_ref.root_type {
                    Some(
                        ::substrait::proto::expression::field_reference::RootType::OuterReference(
                            o,
                        ),
                    ) => Some(o),
                    _ => None,
                };
                self.print_direct_reference(direct_ref, outer_ref)
            }
            Some(ReferenceType::MaskedReference(_)) => {
                Ok("MASKED_REFERENCE_NOT_YET_IMPLEMENTED".to_string())
            }
            None => Err(TextPlanError::InvalidExpression(
                "FieldReference has no reference_type".to_string(),
            )),
        }
    }

    /// Prints a direct reference.
    fn print_direct_reference(
        &self,
        direct_ref: &::substrait::proto::expression::ReferenceSegment,
        outer_ref: Option<&::substrait::proto::expression::field_reference::OuterReference>,
    ) -> Result<String, TextPlanError> {
        use ::substrait::proto::expression::reference_segment::ReferenceType;

        match &direct_ref.reference_type {
            Some(ReferenceType::StructField(struct_field)) => {
                let field_index = struct_field.field as usize;
                self.lookup_field_reference(field_index, outer_ref)
            }
            Some(ReferenceType::MapKey(_)) => {
                Ok("MAP_KEY_REFERENCE_NOT_YET_IMPLEMENTED".to_string())
            }
            Some(ReferenceType::ListElement(_)) => {
                Ok("LIST_ELEMENT_REFERENCE_NOT_YET_IMPLEMENTED".to_string())
            }
            None => Err(TextPlanError::InvalidExpression(
                "DirectReference has no reference_type".to_string(),
            )),
        }
    }

    /// Looks up a field reference in the current scope or outer scope.
    fn lookup_field_reference(
        &self,
        field_index: usize,
        outer_ref: Option<&::substrait::proto::expression::field_reference::OuterReference>,
    ) -> Result<String, TextPlanError> {
        // Handle outer references by looking up the parent scope
        let parent_scope_arc: Option<Arc<SymbolInfo>>;
        let scope: &Arc<SymbolInfo> = if let Some(outer) = outer_ref {
            if outer.steps_out > 0 {
                // Get the parent scope by looking up the parent query location
                let current_scope = self.current_scope.ok_or_else(|| {
                    TextPlanError::InvalidExpression(
                        "Outer reference requested but no current scope".to_string(),
                    )
                })?;

                // Check if current scope has parent info directly, or needs to use pipeline_start
                let (parent_location, parent_index) = if current_scope.parent_query_index() >= 0 {
                    // This relation has parent info directly (it's the terminus)
                    (
                        current_scope.parent_query_location(),
                        current_scope.parent_query_index(),
                    )
                } else {
                    // This relation might be in a subquery pipeline - check pipeline_start
                    if let Some(blob_lock) = &current_scope.blob {
                        if let Ok(blob_data) = blob_lock.lock() {
                            if let Some(relation_data) = blob_data.downcast_ref::<crate::textplan::common::structured_symbol_data::RelationData>() {
                                if let Some(pipeline_start) = &relation_data.pipeline_start {
                                    // Use the terminus's parent info
                                    (pipeline_start.parent_query_location(), pipeline_start.parent_query_index())
                                } else {
                                    // Not in a subquery, use current scope
                                    (current_scope.parent_query_location(), current_scope.parent_query_index())
                                }
                            } else {
                                (current_scope.parent_query_location(), current_scope.parent_query_index())
                            }
                        } else {
                            (
                                current_scope.parent_query_location(),
                                current_scope.parent_query_index(),
                            )
                        }
                    } else {
                        (
                            current_scope.parent_query_location(),
                            current_scope.parent_query_index(),
                        )
                    }
                };

                parent_scope_arc = if parent_index >= 0 {
                    // Look up the parent relation symbol
                    self.symbol_table.lookup_symbol_by_location_and_type(
                        parent_location.as_ref(),
                        SymbolType::Relation,
                    )
                } else {
                    None
                };

                if let Some(ref parent) = parent_scope_arc {
                    parent
                } else {
                    return Err(TextPlanError::InvalidExpression(format!(
                        "Could not find parent scope for outer reference (steps_out={})",
                        outer.steps_out
                    )));
                }
            } else {
                self.current_scope.ok_or_else(|| {
                    TextPlanError::InvalidExpression(
                        "Field reference requested outside of a relation scope".to_string(),
                    )
                })?
            }
        } else {
            self.current_scope.ok_or_else(|| {
                TextPlanError::InvalidExpression(
                    "Field reference requested outside of a relation scope".to_string(),
                )
            })?
        };

        // Get the relation data from the scope's blob
        if let Some(blob_lock) = &scope.blob {
            if let Ok(blob_data) = blob_lock.lock() {
                if let Some(relation_data) = blob_data.downcast_ref::<RelationData>() {
                    // First check in field_references
                    if field_index < relation_data.field_references.len() {
                        let symbol = &relation_data.field_references[field_index];
                        return Ok(self.format_field_name(symbol));
                    }

                    // Then check in generated_field_references
                    let adjusted_index = field_index - relation_data.field_references.len();
                    if adjusted_index < relation_data.generated_field_references.len() {
                        let symbol = &relation_data.generated_field_references[adjusted_index];
                        return Ok(self.format_field_name(symbol));
                    }

                    return Err(TextPlanError::InvalidExpression(format!(
                        "Field reference {} out of range (have {} + {} fields)",
                        field_index,
                        relation_data.field_references.len(),
                        relation_data.generated_field_references.len()
                    )));
                }
            }
        }

        Err(TextPlanError::InvalidExpression(
            "Could not access relation data for field reference lookup".to_string(),
        ))
    }

    /// Formats a field name, using fully qualified name if available.
    fn format_field_name(&self, symbol: &Arc<SymbolInfo>) -> String {
        // If the symbol has an alias, use it
        if let Some(alias) = symbol.alias() {
            return alias.to_string();
        }

        // Otherwise, use fully qualified name if schema is available
        if let Some(schema) = symbol.schema() {
            format!("{}.{}", schema.name(), symbol.name())
        } else {
            symbol.name().to_string()
        }
    }

    /// Looks up a function name by its reference anchor.
    fn lookup_function_reference(&self, function_reference: u32) -> String {
        // Search for a function symbol with matching anchor
        for symbol in self.symbol_table.symbols() {
            if symbol.symbol_type() != SymbolType::Function {
                continue;
            }

            // Try to get the function data from the blob
            if let Some(blob_lock) = &symbol.blob {
                if let Ok(blob_data) = blob_lock.lock() {
                    if let Some(function_data) = blob_data.downcast_ref::<FunctionData>() {
                        if function_data.anchor == function_reference {
                            return symbol.name().to_string();
                        }
                    }
                }
            }
        }

        // Function not found, return a placeholder
        format!("functionref#{}", function_reference)
    }

    /// Prints a scalar function call.
    fn print_scalar_function(
        &mut self,
        func: &::substrait::proto::expression::ScalarFunction,
    ) -> Result<String, TextPlanError> {
        self.function_depth += 1;
        let result = self.print_scalar_function_impl(func);
        self.function_depth -= 1;
        result
    }

    /// Implementation of scalar function printing.
    fn print_scalar_function_impl(
        &mut self,
        func: &::substrait::proto::expression::ScalarFunction,
    ) -> Result<String, TextPlanError> {
        let mut result = String::new();

        // Look up the function name from the symbol table
        let function_name = self.lookup_function_reference(func.function_reference);

        result.push_str(&function_name);
        result.push('(');

        let mut first = true;

        // Print arguments (newer protobuf style)
        for arg in &func.arguments {
            use ::substrait::proto::function_argument::ArgType;
            let arg_str = match &arg.arg_type {
                Some(ArgType::Enum(enum_val)) => {
                    format!("{}_enum", enum_val)
                }
                Some(ArgType::Type(type_val)) => self.print_type(type_val)?,
                Some(ArgType::Value(expr)) => self.print_expression(expr)?,
                None => "MISSING_ARGUMENT".to_string(),
            };

            // Add comma/space separator for non-first arguments
            if !first {
                result.push_str(", ");
            }
            first = false;
            result.push_str(&arg_str);
        }

        // Print args (older protobuf style, for compatibility)
        for arg in &func.args {
            let arg_str = self.print_expression(arg)?;
            // Add comma/space separator for non-first arguments
            if !first {
                result.push_str(", ");
            }
            first = false;
            result.push_str(&arg_str);
        }

        result.push(')');

        // Add return type annotation
        if let Some(output_type) = &func.output_type {
            result.push_str("->");
            result.push_str(&self.print_type(output_type)?);
        }

        Ok(result)
    }

    /// Prints an aggregate function.
    pub fn print_aggregate_function(
        &mut self,
        func: &::substrait::proto::AggregateFunction,
    ) -> Result<String, TextPlanError> {
        self.function_depth += 1;
        let result = self.print_aggregate_function_impl(func);
        self.function_depth -= 1;
        result
    }

    /// Implementation of aggregate function printing.
    fn print_aggregate_function_impl(
        &mut self,
        func: &::substrait::proto::AggregateFunction,
    ) -> Result<String, TextPlanError> {
        let mut result = String::new();

        // Look up the function name from the symbol table
        let function_name = self.lookup_function_reference(func.function_reference);

        result.push_str(&function_name);
        result.push('(');

        let mut first = true;

        // Print arguments (newer protobuf style)
        for arg in &func.arguments {
            use ::substrait::proto::function_argument::ArgType;
            let arg_str = match &arg.arg_type {
                Some(ArgType::Enum(enum_val)) => {
                    format!("{}_enum", enum_val)
                }
                Some(ArgType::Type(type_val)) => self.print_type(type_val)?,
                Some(ArgType::Value(expr)) => self.print_expression(expr)?,
                None => "MISSING_ARGUMENT".to_string(),
            };

            // Add comma/space separator for non-first arguments
            if !first {
                result.push_str(", ");
            }
            first = false;
            result.push_str(&arg_str);
        }

        // Print args (older protobuf style, for compatibility)
        for arg in &func.args {
            let arg_str = self.print_expression(arg)?;
            // Add comma/space separator for non-first arguments
            if !first {
                result.push_str(", ");
            }
            first = false;
            result.push_str(&arg_str);
        }

        result.push(')');

        // Add options (if any)
        for option in &func.options {
            result.push('#');
            result.push_str(&option.name);
            for pref in &option.preference {
                result.push(';');
                result.push_str(pref);
            }
        }

        // Add return type annotation
        if let Some(output_type) = &func.output_type {
            result.push_str("->");
            result.push_str(&self.print_type(output_type)?);
        }

        // Add aggregation phase
        result.push('@');
        let phase_name = match func.phase {
            0 => "AGGREGATION_PHASE_UNSPECIFIED",
            1 => "AGGREGATION_PHASE_INITIAL_TO_INTERMEDIATE",
            2 => "AGGREGATION_PHASE_INTERMEDIATE_TO_INTERMEDIATE",
            3 => "AGGREGATION_PHASE_INITIAL_TO_RESULT",
            4 => "AGGREGATION_PHASE_INTERMEDIATE_TO_RESULT",
            _ => "UNKNOWN_PHASE",
        };
        result.push_str(phase_name);

        Ok(result)
    }

    /// Prints a type annotation.
    pub fn print_type(&self, type_val: &::substrait::proto::Type) -> Result<String, TextPlanError> {
        use ::substrait::proto::r#type::Kind;

        let mut result = String::new();

        let (base_type, nullable) = match &type_val.kind {
            Some(Kind::Bool(bool_type)) => (
                "bool",
                bool_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::I8(i8_type)) => (
                "i8",
                i8_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::I16(i16_type)) => (
                "i16",
                i16_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::I32(i32_type)) => (
                "i32",
                i32_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::I64(i64_type)) => (
                "i64",
                i64_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::Fp32(fp32_type)) => (
                "fp32",
                fp32_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::Fp64(fp64_type)) => (
                "fp64",
                fp64_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::String(string_type)) => (
                "string",
                string_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::Binary(binary_type)) => (
                "binary",
                binary_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::Timestamp(ts_type)) => (
                "timestamp",
                ts_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::Date(date_type)) => (
                "date",
                date_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::Time(time_type)) => (
                "time",
                time_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::IntervalYear(interval_type)) => (
                "interval_year",
                interval_type.nullability
                    == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::IntervalDay(interval_type)) => (
                "interval_day",
                interval_type.nullability
                    == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::TimestampTz(ts_type)) => (
                "timestamp_tz",
                ts_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::Uuid(uuid_type)) => (
                "uuid",
                uuid_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32,
            ),
            Some(Kind::FixedChar(fc_type)) => {
                result.push_str("fixedchar");
                if fc_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32 {
                    result.push('?');
                }
                result.push_str(&format!("<{}>", fc_type.length));
                return Ok(result);
            }
            Some(Kind::Varchar(vc_type)) => {
                result.push_str("varchar");
                if vc_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32 {
                    result.push('?');
                }
                result.push_str(&format!("<{}>", vc_type.length));
                return Ok(result);
            }
            Some(Kind::FixedBinary(fb_type)) => {
                result.push_str("fixedbinary");
                if fb_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32 {
                    result.push('?');
                }
                result.push_str(&format!("<{}>", fb_type.length));
                return Ok(result);
            }
            Some(Kind::Decimal(dec_type)) => {
                result.push_str("decimal");
                if dec_type.nullability == ::substrait::proto::r#type::Nullability::Nullable as i32
                {
                    result.push('?');
                }
                result.push_str(&format!("<{},{}>", dec_type.precision, dec_type.scale));
                return Ok(result);
            }
            Some(Kind::Struct(_)) => return Ok("STRUCT_TYPE_NOT_YET_IMPLEMENTED".to_string()),
            Some(Kind::List(_)) => return Ok("LIST_TYPE_NOT_YET_IMPLEMENTED".to_string()),
            Some(Kind::Map(_)) => return Ok("MAP_TYPE_NOT_YET_IMPLEMENTED".to_string()),
            Some(Kind::UserDefined(_)) => {
                return Ok("USER_DEFINED_TYPE_NOT_YET_IMPLEMENTED".to_string())
            }
            Some(Kind::UserDefinedTypeReference(_)) => {
                return Ok("USER_DEFINED_TYPE_REF_NOT_YET_IMPLEMENTED".to_string())
            }
            Some(Kind::IntervalCompound(_)) => {
                return Ok("INTERVAL_COMPOUND_TYPE_NOT_YET_IMPLEMENTED".to_string())
            }
            Some(Kind::PrecisionTime(_)) => {
                return Ok("PRECISION_TIME_TYPE_NOT_YET_IMPLEMENTED".to_string())
            }
            Some(Kind::PrecisionTimestamp(_)) => {
                return Ok("PRECISION_TIMESTAMP_TYPE_NOT_YET_IMPLEMENTED".to_string())
            }
            Some(Kind::PrecisionTimestampTz(_)) => {
                return Ok("PRECISION_TIMESTAMP_TZ_TYPE_NOT_YET_IMPLEMENTED".to_string())
            }
            Some(Kind::Alias(_)) => return Ok("ALIAS_TYPE_NOT_YET_IMPLEMENTED".to_string()),
            None => {
                return Err(TextPlanError::InvalidExpression(
                    "Type has no kind".to_string(),
                ))
            }
        };

        result.push_str(base_type);
        if nullable {
            result.push('?');
        }

        Ok(result)
    }

    /// Prints an if-then expression.
    fn print_if_then(
        &mut self,
        if_then: &::substrait::proto::expression::IfThen,
    ) -> Result<String, TextPlanError> {
        let mut result = String::from("IFTHEN(");

        let mut first = true;
        for clause in &if_then.ifs {
            if !first {
                result.push_str(", ");
            }
            first = false;

            if let Some(if_expr) = &clause.r#if {
                result.push_str(&self.print_expression(if_expr)?);
            } else {
                result.push_str("MISSING_IF");
            }

            result.push_str(", ");

            if let Some(then_expr) = &clause.then {
                result.push_str(&self.print_expression(then_expr)?);
            } else {
                result.push_str("MISSING_THEN");
            }
        }

        if let Some(else_expr) = &if_then.r#else {
            if !first {
                result.push_str(", ");
            }
            result.push_str(&self.print_expression(else_expr)?);
        }

        result.push(')');
        Ok(result)
    }

    /// Prints a cast expression.
    fn print_cast(
        &mut self,
        cast: &::substrait::proto::expression::Cast,
    ) -> Result<String, TextPlanError> {
        let mut result = String::new();

        if let Some(input) = &cast.input {
            result.push_str(&self.print_expression(input)?);
        } else {
            result.push_str("MISSING_CAST_INPUT");
        }

        result.push_str(" AS ");

        if let Some(type_val) = &cast.r#type {
            result.push_str(&self.print_type(type_val)?);
        } else {
            result.push_str("MISSING_CAST_TYPE");
        }

        Ok(result)
    }

    fn print_subquery(
        &mut self,
        subquery: &::substrait::proto::expression::Subquery,
    ) -> Result<String, TextPlanError> {
        use ::substrait::proto::expression::subquery::SubqueryType;

        match &subquery.subquery_type {
            Some(SubqueryType::SetComparison(set_comp)) => {
                self.print_set_comparison_subquery(set_comp)
            }
            Some(SubqueryType::Scalar(scalar)) => self.print_scalar_subquery(scalar),
            Some(SubqueryType::InPredicate(in_pred)) => self.print_in_predicate_subquery(in_pred),
            Some(SubqueryType::SetPredicate(set_pred)) => {
                self.print_set_predicate_subquery(set_pred)
            }
            None => Err(TextPlanError::InvalidExpression(
                "Subquery has no subquery_type".to_string(),
            )),
        }
    }

    fn print_scalar_subquery(
        &mut self,
        scalar: &::substrait::proto::expression::subquery::Scalar,
    ) -> Result<String, TextPlanError> {
        let mut result = String::new();

        // Print SUBQUERY keyword
        result.push_str("SUBQUERY ");

        // Find the subquery relation symbol
        if let Some(_input) = &scalar.input {
            // Look up the relation symbol for this subquery
            if let Some(scope) = self.current_scope {
                println!(
                    "DEBUG PRINTER: Looking for scalar subquery with parent location hash: {}, index: {}",
                    scope.source_location().location_hash(),
                    self.current_scope_index
                );

                let symbol = self.symbol_table.lookup_symbol_by_parent_query_and_type(
                    scope.source_location(),
                    self.current_scope_index,
                    crate::textplan::SymbolType::Relation,
                );
                self.current_scope_index += 1;

                if let Some(sym) = symbol {
                    println!(
                        "DEBUG PRINTER: Found scalar subquery relation: {}",
                        sym.name()
                    );
                    result.push_str(sym.name());
                } else {
                    return Err(TextPlanError::InvalidExpression(
                        "Could not find scalar subquery relation symbol".to_string(),
                    ));
                }
            } else {
                return Err(TextPlanError::InvalidExpression(
                    "No current scope for scalar subquery lookup".to_string(),
                ));
            }
        } else {
            return Err(TextPlanError::InvalidExpression(
                "Scalar subquery has no input relation".to_string(),
            ));
        }

        Ok(result)
    }

    fn print_in_predicate_subquery(
        &mut self,
        in_pred: &::substrait::proto::expression::subquery::InPredicate,
    ) -> Result<String, TextPlanError> {
        let mut result = String::new();

        // Print needle expressions (left-hand side)
        // Grammar requires: expression_list IN SUBQUERY relation_ref
        // where expression_list is: LEFTPAREN expression ( COMMA expression )* RIGHTPAREN
        // So we always need parentheses, even for single expression
        if in_pred.needles.is_empty() {
            return Err(TextPlanError::InvalidExpression(
                "InPredicate has no needle expressions".to_string(),
            ));
        }

        result.push('(');
        for (i, needle) in in_pred.needles.iter().enumerate() {
            if i > 0 {
                result.push_str(", ");
            }
            result.push_str(&self.print_expression(needle)?);
        }
        result.push(')');

        // Print IN SUBQUERY
        result.push_str(" IN SUBQUERY ");

        // Find the haystack subquery relation symbol
        if let Some(_haystack) = &in_pred.haystack {
            // Look up the relation symbol for this subquery
            if let Some(scope) = self.current_scope {
                let symbol = self.symbol_table.lookup_symbol_by_parent_query_and_type(
                    scope.source_location(),
                    self.current_scope_index,
                    crate::textplan::SymbolType::Relation,
                );
                self.current_scope_index += 1;

                if let Some(sym) = symbol {
                    result.push_str(sym.name());
                } else {
                    return Err(TextPlanError::InvalidExpression(
                        "Could not find IN predicate subquery relation symbol".to_string(),
                    ));
                }
            } else {
                return Err(TextPlanError::InvalidExpression(
                    "No current scope for IN predicate subquery lookup".to_string(),
                ));
            }
        } else {
            return Err(TextPlanError::InvalidExpression(
                "InPredicate has no haystack relation".to_string(),
            ));
        }

        Ok(result)
    }

    fn print_set_predicate_subquery(
        &mut self,
        set_pred: &::substrait::proto::expression::subquery::SetPredicate,
    ) -> Result<String, TextPlanError> {
        use ::substrait::proto::expression::subquery::set_predicate::PredicateOp;

        let mut result = String::new();

        // Print predicate operator (EXISTS or UNIQUE)
        let predicate_op = PredicateOp::try_from(set_pred.predicate_op).map_err(|_| {
            TextPlanError::InvalidExpression(format!(
                "Invalid predicate_op: {}",
                set_pred.predicate_op
            ))
        })?;
        match predicate_op {
            PredicateOp::Exists => result.push_str("EXISTS IN "),
            PredicateOp::Unique => result.push_str("UNIQUE IN "),
            PredicateOp::Unspecified => result.push_str("UNSPECIFIED IN "),
        }

        // Print SUBQUERY keyword
        result.push_str("SUBQUERY ");

        // Find the subquery relation symbol
        if let Some(_tuples) = &set_pred.tuples {
            // Look up the relation symbol for this subquery
            if let Some(scope) = self.current_scope {
                println!(
                    "DEBUG PRINTER: Looking for SET predicate subquery with parent location hash: {}, index: {}",
                    scope.source_location().location_hash(),
                    self.current_scope_index
                );

                let symbol = self.symbol_table.lookup_symbol_by_parent_query_and_type(
                    scope.source_location(),
                    self.current_scope_index,
                    crate::textplan::SymbolType::Relation,
                );
                self.current_scope_index += 1;

                if let Some(sym) = symbol {
                    println!(
                        "DEBUG PRINTER: Found SET predicate subquery relation: {}",
                        sym.name()
                    );
                    result.push_str(sym.name());
                } else {
                    println!(
                        "DEBUG PRINTER: Could NOT find SET predicate subquery with parent hash: {}, index: {}",
                        scope.source_location().location_hash(),
                        self.current_scope_index - 1
                    );
                    return Err(TextPlanError::InvalidExpression(
                        "Could not find SET predicate subquery relation symbol".to_string(),
                    ));
                }
            } else {
                return Err(TextPlanError::InvalidExpression(
                    "No current scope for SET predicate subquery lookup".to_string(),
                ));
            }
        } else {
            return Err(TextPlanError::InvalidExpression(
                "SetPredicate has no tuples relation".to_string(),
            ));
        }

        Ok(result)
    }

    fn print_set_comparison_subquery(
        &mut self,
        set_comp: &::substrait::proto::expression::subquery::SetComparison,
    ) -> Result<String, TextPlanError> {
        use ::substrait::proto::expression::subquery::set_comparison::{ComparisonOp, ReductionOp};

        let mut result = String::new();

        // Print left expression
        if let Some(left) = &set_comp.left {
            result.push_str(&self.print_expression(left)?);
            result.push(' ');
        } else {
            return Err(TextPlanError::InvalidExpression(
                "SetComparison has no left expression".to_string(),
            ));
        }

        // Print comparison operator
        let comp_op = ComparisonOp::try_from(set_comp.comparison_op).map_err(|_| {
            TextPlanError::InvalidExpression(format!(
                "Invalid comparison_op: {}",
                set_comp.comparison_op
            ))
        })?;
        match comp_op {
            ComparisonOp::Unspecified => result.push_str("UNSPECIFIED "),
            ComparisonOp::Eq => result.push_str("EQ "),
            ComparisonOp::Ne => result.push_str("NE "),
            ComparisonOp::Lt => result.push_str("LT "),
            ComparisonOp::Gt => result.push_str("GT "),
            ComparisonOp::Le => result.push_str("LE "),
            ComparisonOp::Ge => result.push_str("GE "),
        }

        // Print reduction operator
        let reduction_op = ReductionOp::try_from(set_comp.reduction_op).map_err(|_| {
            TextPlanError::InvalidExpression(format!(
                "Invalid reduction_op: {}",
                set_comp.reduction_op
            ))
        })?;
        match reduction_op {
            ReductionOp::Unspecified => result.push_str("UNSPECIFIED "),
            ReductionOp::Any => result.push_str("ANY "),
            ReductionOp::All => result.push_str("ALL "),
        }

        // Print SUBQUERY keyword
        result.push_str("SUBQUERY ");

        // Find the subquery relation symbol
        if let Some(_right) = &set_comp.right {
            // Look up the relation symbol for this subquery
            if let Some(scope) = self.current_scope {
                println!(
                    "DEBUG PRINTER: Looking for subquery with parent location hash: {}, index: {}",
                    scope.source_location().location_hash(),
                    self.current_scope_index
                );

                // Debug: list all relations with parent query info
                for symbol in self.symbol_table.symbols() {
                    if symbol.symbol_type() == crate::textplan::SymbolType::Relation {
                        let parent_loc = symbol.parent_query_location();
                        println!(
                            "  - '{}' parent_hash={}, parent_index={}",
                            symbol.name(),
                            parent_loc.location_hash(),
                            symbol.parent_query_index()
                        );
                    }
                }

                let symbol = self.symbol_table.lookup_symbol_by_parent_query_and_type(
                    scope.source_location(),
                    self.current_scope_index,
                    crate::textplan::SymbolType::Relation,
                );
                self.current_scope_index += 1;

                if let Some(sym) = symbol {
                    result.push_str(sym.name());
                } else {
                    return Err(TextPlanError::InvalidExpression(
                        "Could not find subquery relation symbol".to_string(),
                    ));
                }
            } else {
                return Err(TextPlanError::InvalidExpression(
                    "No current scope for subquery lookup".to_string(),
                ));
            }
        } else {
            return Err(TextPlanError::InvalidExpression(
                "SetComparison has no right relation".to_string(),
            ));
        }

        Ok(result)
    }
}

/// Escapes a string for output in textplan format.
fn escape_string(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            _ => result.push(c),
        }
    }
    result
}
