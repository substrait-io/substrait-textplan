// SPDX-License-Identifier: Apache-2.0

//! Error listener for the ANTLR4 parser.

use std::fmt;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

// Import everything we need from antlr_rust
use crate::textplan::common::text_location::TextLocation;
use antlr_rust::atn_config_set::ATNConfigSet;
use antlr_rust::dfa::DFA;
use antlr_rust::token_factory::{TokenAware, TokenFactory};

/// Represents an error encountered during parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// The message describing the error.
    message: String,
    /// The location where the error occurred.
    location: TextLocation,
}

impl ParseError {
    /// Creates a new parse error.
    pub fn new(message: String, location: TextLocation) -> Self {
        Self { message, location }
    }

    /// Returns the message describing the error.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the location where the error occurred.
    pub fn location(&self) -> TextLocation {
        self.location
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}", self.message, self.location)
    }
}

/// Listens for errors during parsing.
pub struct ErrorListener {
    /// The errors encountered during parsing.
    errors: Arc<Mutex<Vec<ParseError>>>,
}

impl Default for ErrorListener {
    fn default() -> Self {
        Self::new()
    }
}

impl ErrorListener {
    /// Creates a new error listener.
    pub fn new() -> Self {
        Self {
            errors: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Adds an error to the listener.
    pub fn add_error(&self, message: String, location: TextLocation) {
        let mut errors = self.errors.lock().unwrap();
        errors.push(ParseError::new(message, location));
    }

    /// Returns true if any errors were encountered during parsing.
    pub fn has_errors(&self) -> bool {
        let errors = self.errors.lock().unwrap();
        !errors.is_empty()
    }

    /// Formats all errors as strings.
    pub fn format_errors(&self) -> Vec<String> {
        let errors = self.errors.lock().unwrap();
        errors.iter().map(|e| e.to_string()).collect()
    }
}

/// Implementation of ANTLR's error listener for our parser.
/// This adapts ANTLR's error reporting to our ErrorListener.
pub struct AntlrErrorListener {
    error_listener: Arc<ErrorListener>,
}

impl AntlrErrorListener {
    /// Creates a new ANTLR error listener wrapping our ErrorListener
    pub fn new(error_listener: Arc<ErrorListener>) -> Self {
        Self { error_listener }
    }

    /// Reports a syntax error.
    pub fn syntax_error(&self, line: isize, column: isize, msg: &str) {
        // Convert line/column to our Location type and add the error
        let location = TextLocation::new(line as i32, column as i32);
        self.error_listener.add_error(msg.to_string(), location);
    }

    /// Gets the underlying ErrorListener
    pub fn get_error_listener(&self) -> Arc<ErrorListener> {
        self.error_listener.clone()
    }
}

// The error listener interface has changed with the antlr-rust 0.3.0-beta version.
// We need to provide a generic implementation that works with ANTLR's recognizer types.
impl<'input, T> antlr_rust::error_listener::ErrorListener<'input, T> for AntlrErrorListener
where
    T: TokenAware<'input> + antlr_rust::recognizer::Recognizer<'input>,
{
    fn syntax_error(
        &self,
        _recognizer: &T,
        _offending_symbol: std::option::Option<
            &<<T as TokenAware<'input>>::TF as TokenFactory<'input>>::Inner,
        >,
        line: isize,
        column: isize,
        msg: &str,
        _e: std::option::Option<&antlr_rust::errors::ANTLRError>,
    ) {
        // Forward to our internal method
        self.syntax_error(line, column, msg);
    }

    fn report_ambiguity(
        &self,
        _recognizer: &T,
        _dfa: &DFA,
        start_index: isize,
        stop_index: isize,
        _exact: bool,
        _ambig_alts: &bit_set::BitSet,
        _configs: &ATNConfigSet,
    ) {
        // Treat ambiguity as a warning, not an error
        // ANTLR automatically resolves ambiguities, so this is just informational
        // The C++ version likely logs these but doesn't fail the parse
        // Uncomment below to log ambiguities for debugging:
        // eprintln!("Warning: Grammar ambiguity at indices {}-{}", start_index, stop_index);
    }

    fn report_attempting_full_context(
        &self,
        _recognizer: &T,
        _dfa: &DFA,
        start_index: isize,
        stop_index: isize,
        _conflict_alts: &bit_set::BitSet,
        _configs: &ATNConfigSet,
    ) {
        // Treat full context attempts as informational, not errors
        // This is normal ANTLR behavior when resolving ambiguities
        // Uncomment below to log for debugging:
        // eprintln!("Info: Parser attempting full context parsing at indices {}-{}", start_index, stop_index);
    }

    fn report_context_sensitivity(
        &self,
        _recognizer: &T,
        _dfa: &DFA,
        start_index: isize,
        stop_index: isize,
        prediction: isize,
        _configs: &ATNConfigSet,
    ) {
        // Log context sensitivity
        self.error_listener.add_error(
            format!(
                "Context sensitivity detected at indices {}-{} with prediction {}",
                start_index, stop_index, prediction
            ),
            TextLocation::new(start_index as i32, stop_index as i32),
        );
    }
}

/// Creates a Box-wrapped version of AntlrErrorListener suitable for adding to ANTLR parsers
pub fn create_boxed_error_listener(error_listener: Arc<ErrorListener>) -> Box<AntlrErrorListener> {
    Box::new(AntlrErrorListener::new(error_listener))
}
