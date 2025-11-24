// SPDX-License-Identifier: Apache-2.0

//! Parser for Substrait textplans.

pub mod error_listener;
pub mod grammar;
pub mod load_text;
pub mod parse_text;
pub mod plan_visitor;
// ANTLR visitors
pub mod visitors;
// Generated ANTLR code
pub mod antlr;

// Re-export the main functions
pub use load_text::load_from_text;
pub use parse_text::{load_text_file, load_text_string, parse_stream};
