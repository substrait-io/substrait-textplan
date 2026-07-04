// SPDX-License-Identifier: Apache-2.0

//! Common utilities and types for the textplan module.

pub mod error;
pub mod location;
pub mod parse_result;
pub mod proto_location;
pub mod string_utils;
pub mod structured_symbol_data;
pub mod text_location;
pub mod unknown_location;

// Re-export the trait and concrete location types
pub use location::BoxedLocation;
pub use location::Location;
pub use proto_location::ProtoLocation;
pub use text_location::TextLocation;
pub use unknown_location::UnknownLocation;
