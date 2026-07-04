// SPDX-License-Identifier: Apache-2.0

//! Error types for the textplan module.

use thiserror::Error;

/// Errors that can occur when working with textplans.
#[derive(Error, Debug)]
pub enum TextPlanError {
    /// An error that occurred during parsing.
    #[error("Parse error: {0}")]
    ParseError(String),

    /// An error that occurred during binary conversion.
    #[error("Binary conversion error: {0}")]
    BinaryConversionError(String),

    /// An error that occurred when working with protobuf.
    #[error("Protobuf error: {0}")]
    ProtobufError(String),

    /// An error that occurred when performing I/O.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// An error that occurred when working with the symbol table.
    #[error("Symbol table error: {0}")]
    SymbolTableError(String),

    /// An error that occurred when printing an expression.
    #[error("Invalid expression: {0}")]
    InvalidExpression(String),
}

/// Result type for textplan operations.
pub type Result<T> = std::result::Result<T, TextPlanError>;
