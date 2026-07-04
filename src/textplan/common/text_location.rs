// SPDX-License-Identifier: Apache-2.0

//! The text_location module provides a way to track source positions in the parsed text.

use crate::textplan::common::Location;
use std::any::Any;
use std::fmt;

/// Represents a position in the source text.
///
/// This is used to provide context for errors and to reference entities in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextLocation {
    /// The position in the source text, 0-indexed.
    position: i32,
    /// The length of the text at this location.
    length: i32,
}

impl TextLocation {
    /// A special location used to indicate an unknown position.
    pub const UNKNOWN_LOCATION: TextLocation = TextLocation {
        position: -1,
        length: 0,
    };

    /// Creates a text location corresponding to the global unknown location.
    pub fn unknown() -> Self {
        Self::UNKNOWN_LOCATION
    }

    /// Creates a new location from a position and length.
    pub fn new(position: i32, length: i32) -> Self {
        Self { position, length }
    }

    /// Returns the position in the source text.
    pub fn position(&self) -> i32 {
        self.position
    }

    /// Returns the length of the text at this location.
    pub fn length(&self) -> i32 {
        self.length
    }
}

impl Location for TextLocation {
    fn is_unknown(&self) -> bool {
        self.position < 0
    }

    // Implement location_hash
    fn location_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }

    // Create a boxed clone of this location
    fn box_clone(&self) -> Box<dyn Location> {
        Box::new(*self)
    }

    // Convert to Any for downcasting
    fn as_any(&self) -> &dyn Any {
        self
    }

    // Custom equals implementation
    fn equals(&self, other: &dyn Location) -> bool {
        // First check if other is a TextLocation using Any
        if let Some(other_text) = other.as_any().downcast_ref::<TextLocation>() {
            // Compare text locations directly
            self == other_text
        } else {
            // Different types, so not equal
            false
        }
    }
}

impl fmt::Display for TextLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_unknown() {
            write!(f, "unknown")
        } else {
            write!(f, "pos {} len {}", self.position, self.length)
        }
    }
}

impl Default for TextLocation {
    fn default() -> Self {
        Self::UNKNOWN_LOCATION
    }
}
