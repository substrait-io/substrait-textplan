// SPDX-License-Identifier: Apache-2.0

//! Defines an unknown location implementation.

use crate::textplan::common::Location;
use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};

/// A type that represents an unknown location.
///
/// This provides a concrete implementation of Location that represents
/// an unknown or unspecified location, which can be used across the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownLocation;

impl UnknownLocation {
    /// A global constant for the unknown location
    pub const UNKNOWN: UnknownLocation = UnknownLocation;

    /// Returns a boxed unknown location that implements the Location trait.
    pub fn boxed() -> Box<dyn Location> {
        Box::new(UnknownLocation)
    }

    /// Returns a unique, consistent hash value for the unknown location.
    /// This value is chosen to be unlikely to conflict with other location hashes.
    pub fn unique_hash() -> u64 {
        // A randomly chosen value that's unlikely to conflict with other hashes
        0xDEADBEEF_DEADBEEF
    }
}

impl Location for UnknownLocation {
    fn is_unknown(&self) -> bool {
        true
    }

    // Override the default hash implementation to use our unique hash
    fn location_hash(&self) -> u64 {
        Self::unique_hash()
    }

    fn box_clone(&self) -> Box<dyn Location> {
        Box::new(*self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Custom equals implementation - UnknownLocation is only equal to itself
    fn equals(&self, other: &dyn Location) -> bool {
        other.is_unknown() && other.as_any().is::<UnknownLocation>()
    }
}

impl Default for UnknownLocation {
    fn default() -> Self {
        UnknownLocation
    }
}

impl fmt::Display for UnknownLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown location")
    }
}

// Custom Hash implementation to ensure UnknownLocation always gives the same hash value
impl Hash for UnknownLocation {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Use our unique hash value instead of the default
        state.write_u64(Self::unique_hash());
    }
}
