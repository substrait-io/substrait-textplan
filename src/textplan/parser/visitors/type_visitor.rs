// SPDX-License-Identifier: Apache-2.0

//! Type visitor for processing type definitions in the parse tree.

use std::sync::Arc;

use antlr_rust::parser_rule_context::ParserRuleContext;
use antlr_rust::rule_context::RuleContext;
use antlr_rust::token::{GenericToken, Token};
use antlr_rust::tree::{ParseTree, ParseTreeVisitor};

use crate::textplan::parser::antlr::substraitplanparser::*;
use crate::textplan::parser::antlr::substraitplanparservisitor::SubstraitPlanParserVisitor;
use crate::textplan::parser::error_listener::ErrorListener;
use crate::textplan::symbol_table::SymbolTable;
use ::substrait::proto::r#type::{
    Binary, Boolean, Date, Decimal, FixedBinary, FixedChar, Fp32, Fp64, IntervalDay, IntervalYear,
    Kind, List, Map, Nullability, String as StringType, Struct, Time, Timestamp, TimestampTz, Uuid,
    VarChar, I16, I32, I64, I8,
};
use ::substrait::proto::Type;

use super::{token_to_location, BasePlanVisitor, PlanVisitor};

/// The TypeVisitor processes and validates types in the parse tree.
///
/// This visitor is the first phase in the multiphase parsing approach, focusing on
/// basic and complex types. It builds on the BasePlanVisitor to provide common
/// symbol table and error handling functionality.
pub struct TypeVisitor<'input> {
    base: BasePlanVisitor,
    _phantom: std::marker::PhantomData<&'input ()>,
}

impl<'input> TypeVisitor<'input> {
    /// Creates a new TypeVisitor.
    pub fn new(symbol_table: SymbolTable, error_listener: Arc<ErrorListener>) -> Self {
        Self {
            base: BasePlanVisitor::new(symbol_table, error_listener),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Gets a mutable reference to the symbol table for modifications.
    pub fn symbol_table_mut(&mut self) -> &mut SymbolTable {
        self.base.symbol_table_mut()
    }

    /// Converts a text representation of a type to a Substrait protobuf Type.
    /// This follows the C++ substrait type library logic for handling nullable markers.
    pub fn text_to_type_proto(
        &self,
        ctx: &dyn SubstraitPlanParserContext<'input>,
        type_text: &str,
    ) -> Type {
        let mut proto_type = Type::default();

        // Find positions of special characters
        let question_pos = type_text.find('?');
        let left_angle_pos = type_text.find('<');

        // Determine nullability following C++ logic:
        // - For parameterized types: check if ? appears immediately before <
        // - For simple types: check if last character is ?
        let nullable = if let Some(angle_pos) = left_angle_pos {
            // Parameterized type case: "decimal?<19,0>"
            angle_pos > 0 && question_pos == Some(angle_pos - 1)
        } else {
            // Simple type case: "i32?"
            question_pos == Some(type_text.len() - 1)
        };

        // Extract base type name (without the ? marker)
        let base_type_name = if nullable {
            if let Some(q_pos) = question_pos {
                &type_text[..q_pos]
            } else {
                type_text
            }
        } else {
            type_text
        };

        // Reconstruct the type string for parsing (base name + parameters)
        // For "decimal?<19,0>" this becomes "decimal<19,0>"
        // For "i32?" this becomes "i32"
        let base_type_str = if let Some(angle_pos) = left_angle_pos {
            if nullable {
                // Concatenate base name + everything from < onwards
                format!("{}{}", base_type_name, &type_text[angle_pos..])
            } else {
                type_text.to_string()
            }
        } else {
            base_type_name.to_string()
        };

        let nullability = if nullable {
            Nullability::Nullable
        } else {
            Nullability::Required
        };

        // Parse complex types
        if let Some(list_content) = base_type_str
            .strip_prefix("list<")
            .and_then(|s| s.strip_suffix(">"))
        {
            // List type - format: list<element_type>
            let element_type = self.text_to_type_proto(ctx, list_content);
            let mut list_type = List::default();
            list_type.nullability = nullability.into();
            list_type.r#type = Some(Box::new(element_type));
            proto_type.kind = Some(Kind::List(Box::new(list_type)));
            return proto_type;
        } else if let Some(struct_content) = base_type_str
            .strip_prefix("struct<")
            .and_then(|s| s.strip_suffix(">"))
        {
            // Struct type - format: struct<field1_type, field2_type, ...>
            let mut struct_type = Struct::default();
            struct_type.nullability = nullability.into();

            // Split the struct content by commas, respecting nesting
            let field_types = self.split_struct_fields(struct_content);

            for field_type_str in field_types {
                let field_type = self.text_to_type_proto(ctx, field_type_str.trim());
                struct_type.types.push(field_type);
            }

            proto_type.kind = Some(Kind::Struct(struct_type));
            return proto_type;
        } else if let Some(map_content) = base_type_str
            .strip_prefix("map<")
            .and_then(|s| s.strip_suffix(">"))
        {
            // Map type - format: map<key_type, value_type>
            if let Some((key_type_str, value_type_str)) = map_content.split_once(',') {
                let key_type = self.text_to_type_proto(ctx, key_type_str.trim());
                let value_type = self.text_to_type_proto(ctx, value_type_str.trim());

                let mut map_type = Map::default();
                map_type.nullability = nullability.into();
                map_type.key = Some(Box::new(key_type));
                map_type.value = Some(Box::new(value_type));

                proto_type.kind = Some(Kind::Map(Box::new(map_type)));
                return proto_type;
            } else {
                // Use the start() method properly
                // Get the start token directly - it's not an Option
                let token = ctx.start();
                self.add_error(&token, &format!("Invalid map type format: {}", type_text));
            }
        } else if let Some(decimal_content) = base_type_str
            .strip_prefix("decimal<")
            .and_then(|s| s.strip_suffix(">"))
        {
            // Decimal type - format: decimal<precision, scale>
            if let Some((precision_str, scale_str)) = decimal_content.split_once(',') {
                if let (Ok(precision), Ok(scale)) = (
                    precision_str.trim().parse::<i32>(),
                    scale_str.trim().parse::<i32>(),
                ) {
                    let mut decimal_type = Decimal::default();
                    decimal_type.nullability = nullability.into();
                    decimal_type.precision = precision;
                    decimal_type.scale = scale;

                    proto_type.kind = Some(Kind::Decimal(decimal_type));
                    return proto_type;
                } else {
                    // Get the start token directly - it's not an Option
                    let token = ctx.start();
                    self.add_error(
                        &token,
                        &format!("Invalid decimal parameters: {}", decimal_content),
                    );
                }
            } else {
                // Get the start token directly - it's not an Option
                let token = ctx.start();
                self.add_error(
                    &token,
                    &format!("Invalid decimal type format: {}", type_text),
                );
            }
        } else if let Some(fixed_char_content) = base_type_str
            .strip_prefix("fixedchar<")
            .and_then(|s| s.strip_suffix(">"))
        {
            // Fixed char type - format: fixedchar<length>
            if let Ok(length) = fixed_char_content.trim().parse::<i32>() {
                let mut fixed_char_type = FixedChar::default();
                fixed_char_type.nullability = nullability.into();
                fixed_char_type.length = length;

                proto_type.kind = Some(Kind::FixedChar(fixed_char_type));
                return proto_type;
            } else {
                // Get the start token directly - it's not an Option
                let token = ctx.start();
                self.add_error(
                    &token,
                    &format!("Invalid fixedchar length: {}", fixed_char_content),
                );
            }
        } else if let Some(varchar_content) = base_type_str
            .strip_prefix("varchar<")
            .and_then(|s| s.strip_suffix(">"))
        {
            // Varchar type - format: varchar<length>
            if let Ok(length) = varchar_content.trim().parse::<i32>() {
                let mut varchar_type = VarChar::default();
                varchar_type.nullability = nullability.into();
                varchar_type.length = length;

                proto_type.kind = Some(Kind::Varchar(varchar_type));
                return proto_type;
            } else {
                // Get the start token directly - it's not an Option
                let token = ctx.start();
                self.add_error(
                    &token,
                    &format!("Invalid varchar length: {}", varchar_content),
                );
            }
        } else if let Some(fixed_binary_content) = base_type_str
            .strip_prefix("fixedbinary<")
            .and_then(|s| s.strip_suffix(">"))
        {
            // Fixed binary type - format: fixedbinary<length>
            if let Ok(length) = fixed_binary_content.trim().parse::<i32>() {
                let mut fixed_binary_type = FixedBinary::default();
                fixed_binary_type.nullability = nullability.into();
                fixed_binary_type.length = length;

                proto_type.kind = Some(Kind::FixedBinary(fixed_binary_type));
                return proto_type;
            } else {
                // Get the start token directly - it's not an Option
                let token = ctx.start();
                self.add_error(
                    &token,
                    &format!("Invalid fixedbinary length: {}", fixed_binary_content),
                );
            }
        }

        // Handle basic types
        match base_type_str.as_str() {
            "boolean" | "bool" => {
                let mut boolean = Boolean::default();
                boolean.nullability = nullability.into();
                proto_type.kind = Some(Kind::Bool(boolean));
            }
            "i8" | "int8" | "tinyint" => {
                let mut i8_type = I8::default();
                i8_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::I8(i8_type));
            }
            "i16" | "int16" | "smallint" => {
                let mut i16_type = I16::default();
                i16_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::I16(i16_type));
            }
            "i32" | "int32" | "int" => {
                let mut i32_type = I32::default();
                i32_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::I32(i32_type));
            }
            "i64" | "int64" | "bigint" => {
                let mut i64_type = I64::default();
                i64_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::I64(i64_type));
            }
            "fp32" | "float" => {
                let mut fp32_type = Fp32::default();
                fp32_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::Fp32(fp32_type));
            }
            "fp64" | "double" => {
                let mut fp64_type = Fp64::default();
                fp64_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::Fp64(fp64_type));
            }
            "string" => {
                let mut string_type = StringType::default();
                string_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::String(string_type));
            }
            "binary" => {
                let mut binary_type = Binary::default();
                binary_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::Binary(binary_type));
            }
            "timestamp" => {
                let mut timestamp_type = Timestamp::default();
                timestamp_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::Timestamp(timestamp_type));
            }
            "timestamp_tz" => {
                let mut timestamp_tz_type = TimestampTz::default();
                timestamp_tz_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::TimestampTz(timestamp_tz_type));
            }
            "date" => {
                let mut date_type = Date::default();
                date_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::Date(date_type));
            }
            "time" => {
                let mut time_type = Time::default();
                time_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::Time(time_type));
            }
            "interval_year" | "interval_year_to_month" | "interval_year_month" => {
                let mut interval_year_type = IntervalYear::default();
                interval_year_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::IntervalYear(interval_year_type));
            }
            "interval_day" | "interval_day_to_second" | "interval_day_second" => {
                let mut interval_day_type = IntervalDay::default();
                interval_day_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::IntervalDay(interval_day_type));
            }
            "uuid" => {
                let mut uuid_type = Uuid::default();
                uuid_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::Uuid(uuid_type));
            }
            "fixedchar" => {
                // Bare fixedchar without length parameter (used in literal suffixes like "text"_fixedchar)
                // We don't know the length from the literal alone, so use length 0 as a placeholder
                let mut fixed_char_type = FixedChar::default();
                fixed_char_type.nullability = nullability.into();
                fixed_char_type.length = 0;
                proto_type.kind = Some(Kind::FixedChar(fixed_char_type));
            }
            "varchar" => {
                // Bare varchar without length parameter (used in literal suffixes like "text"_varchar)
                let mut varchar_type = VarChar::default();
                varchar_type.nullability = nullability.into();
                varchar_type.length = 0;
                proto_type.kind = Some(Kind::Varchar(varchar_type));
            }
            // For unknown types, log an error and use i32 as a fallback
            _ => {
                // Get the start token directly - it's not an Option
                let token = ctx.start();
                self.add_error(
                    &token,
                    &format!("Unsupported or unknown type: {}", type_text),
                );
                // Set a default type for now
                let mut unknown_type = I32::default();
                unknown_type.nullability = nullability.into();
                proto_type.kind = Some(Kind::I32(unknown_type));
            }
        }

        proto_type
    }

    /// Helper function to split struct fields, respecting nested angle brackets.
    fn split_struct_fields(&self, struct_content: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut current_field = String::new();
        let mut angle_bracket_depth = 0;

        for c in struct_content.chars() {
            match c {
                '<' => {
                    angle_bracket_depth += 1;
                    current_field.push(c);
                }
                '>' => {
                    angle_bracket_depth -= 1;
                    current_field.push(c);
                }
                ',' if angle_bracket_depth == 0 => {
                    // Only split at commas that are not inside angle brackets
                    result.push(current_field.trim().to_string());
                    current_field.clear();
                }
                _ => {
                    current_field.push(c);
                }
            }
        }

        // Add the last field if there is one
        if !current_field.trim().is_empty() {
            result.push(current_field.trim().to_string());
        }

        result
    }

    /// Determines if the context is inside a struct literal with an external type.
    pub fn inside_struct_literal_with_external_type(
        &self,
        _ctx: &dyn SubstraitPlanParserContext<'input>,
    ) -> bool {
        // This would check up the parse tree to determine context
        // For now, default to false
        false
    }

    /// Adds an error message to the error listener.
    pub fn add_error<'a>(
        &self,
        token: &impl std::ops::Deref<Target = GenericToken<std::borrow::Cow<'a, str>>>,
        message: &str,
    ) {
        let location = token_to_location(token);
        self.base
            .error_listener()
            .add_error(message.to_string(), location);
    }
}

impl<'input> PlanVisitor<'input> for TypeVisitor<'input> {
    fn error_listener(&self) -> Arc<ErrorListener> {
        self.base.error_listener()
    }

    fn symbol_table(&self) -> SymbolTable {
        self.base.symbol_table()
    }
}

// ANTLR visitor implementation for TypeVisitor
impl<'input> ParseTreeVisitor<'input, SubstraitPlanParserContextType> for TypeVisitor<'input> {}

impl<'input> SubstraitPlanParserVisitor<'input> for TypeVisitor<'input> {
    // Override specific visitor methods for literal types
    fn visit_literal_basic_type(&mut self, ctx: &Literal_basic_typeContext<'input>) {
        // In a real implementation, we would process this and store the result
        // For now, we'll just process the type text - using Token trait's get_text method
        let type_text = ctx.get_text();
        let _type_proto = self.text_to_type_proto(ctx, &type_text);

        // Visit children nodes in case there are nested types
        self.visit_children(ctx);
    }

    fn visit_literal_complex_type(&mut self, ctx: &Literal_complex_typeContext<'input>) {
        // In a real implementation, we would process this and store the result
        // For now, we'll just process the type text
        let type_text = ctx.get_text();
        let _type_proto = self.text_to_type_proto(ctx, &type_text);

        // Visit children nodes in case there are nested types
        self.visit_children(ctx);
    }

    // We use the default implementation for other visitor methods,
    // which will call visit_children to traverse the entire tree
}
