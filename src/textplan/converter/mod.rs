// SPDX-License-Identifier: Apache-2.0

//! Converter for binary and JSON Substrait plans.

pub mod generated;
mod initial_plan_visitor;
pub mod load_json;
mod pipeline_visitor;
pub mod save_binary;
pub mod save_json;
pub mod save_text;

// Re-export the main conversion functions
pub use load_json::load_from_json_file;
pub use save_binary::save_to_binary;
pub use save_json::save_to_json;
pub use save_json::save_to_json_file;
pub use save_json::save_to_json_from_text;
pub use save_text::process_plan_with_visitor;
pub use save_text::save_to_text;

// Re-export the visitor traits
pub use generated::PlanProtoVisitor;
pub use generated::Traversable;
