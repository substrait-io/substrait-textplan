// SPDX-License-Identifier: Apache-2.0

//! The location module provides a trait for tracking positions in various contexts.

use std::any::Any;
use std::fmt::Debug;

// Import the location types for the From<T> impls
use crate::textplan::common::proto_location::ProtoLocation;
use crate::textplan::common::text_location::TextLocation;
use crate::textplan::common::unknown_location::UnknownLocation;

/// A trait representing a location in source material.
///
/// This trait provides the essential functionality needed for location objects,
/// and is deliberately kept object-safe to support trait objects.
pub trait Location: Debug + Send + Sync + 'static {
    /// Returns true if this is an unknown location.
    fn is_unknown(&self) -> bool;

    /// Computes a stable hash for the location.
    ///
    /// This is used for location comparison in the symbol table and must be consistent.
    fn location_hash(&self) -> u64;

    /// Creates a boxed clone of this location.
    fn box_clone(&self) -> Box<dyn Location>;

    /// Converts this location to Any for downcasting.
    fn as_any(&self) -> &dyn Any;

    /// Tests if this location is equal to another location.
    fn equals(&self, other: &dyn Location) -> bool {
        // Default implementation uses location hash
        self.location_hash() == other.location_hash()
    }
}

impl Clone for Box<dyn Location> {
    fn clone(&self) -> Self {
        self.box_clone()
    }
}

/// A boxed Location trait object.
///
/// This is a convenience type for working with location objects that
/// may be of different concrete types.
pub type BoxedLocation = Box<dyn Location>;

// Implementation of From<T> for Box<dyn Location> for various types
// This allows locations to be directly passed to methods that expect Into<Box<dyn Location>>

// For &Box<dyn Location>
impl<'a> From<&'a BoxedLocation> for Box<dyn Location> {
    fn from(location: &'a BoxedLocation) -> Self {
        location.box_clone()
    }
}

// For TextLocation
impl From<TextLocation> for Box<dyn Location> {
    fn from(location: TextLocation) -> Self {
        Box::new(location)
    }
}

// For ProtoLocation
impl From<ProtoLocation> for Box<dyn Location> {
    fn from(location: ProtoLocation) -> Self {
        Box::new(location)
    }
}

// For UnknownLocation
impl From<UnknownLocation> for Box<dyn Location> {
    fn from(location: UnknownLocation) -> Self {
        Box::new(location)
    }
}
