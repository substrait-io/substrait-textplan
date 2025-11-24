//! Generated code for Substrait plan visitors.
//!
//! This module re-exports the generated code from the base_plan_visitor.rs file.
//! The build script (build.rs) generates the base_plan_visitor.rs file based on
//! the current Substrait protobuf schema.

// Re-export everything from the generated visitor module
pub use self::base_plan_visitor::*;

// Include the generated code as public
pub mod base_plan_visitor;
