// SPDX-License-Identifier: Apache-2.0

//! Proto comparison utilities for testing, inspired by protobuf_matchers from substrait-cpp.
//!
//! This module provides utilities to compare protobuf messages and report detailed
//! differences, similar to Google's protobuf matchers but implemented in Rust.

use serde_json::Value;
use std::collections::HashSet;

/// A difference found when comparing two protobuf messages.
#[derive(Debug, Clone)]
pub struct ProtoDifference {
    /// The JSON path to the differing field (e.g., "relations[0].root.input.read.baseSchema")
    pub path: String,
    /// The value in the expected proto (as JSON)
    pub expected: String,
    /// The value in the actual proto (as JSON)
    pub actual: String,
}

impl ProtoDifference {
    fn new(path: String, expected: String, actual: String) -> Self {
        Self {
            path,
            expected,
            actual,
        }
    }
}

/// Configuration for proto comparison.
#[derive(Debug, Clone)]
pub struct ProtoMatcherConfig {
    /// Maximum number of differences to report before stopping
    pub max_differences: usize,
    /// Set of field paths to ignore during comparison (e.g., "version.minorNumber")
    pub ignored_fields: HashSet<String>,
}

impl Default for ProtoMatcherConfig {
    fn default() -> Self {
        Self {
            max_differences: 10,
            ignored_fields: HashSet::new(),
        }
    }
}

impl ProtoMatcherConfig {
    /// Create a new config that ignores the version field
    pub fn ignoring_version() -> Self {
        let mut ignored_fields = HashSet::new();
        ignored_fields.insert("version".to_string());
        ignored_fields.insert("version.minorNumber".to_string());
        ignored_fields.insert("version.producer".to_string());
        Self {
            max_differences: 10,
            ignored_fields,
        }
    }

    /// Add a field path to ignore
    pub fn ignore_field(mut self, path: &str) -> Self {
        self.ignored_fields.insert(path.to_string());
        self
    }

    /// Set the maximum number of differences to report
    pub fn with_max_differences(mut self, max: usize) -> Self {
        self.max_differences = max;
        self
    }
}

/// Compare two Plan protos and return a list of differences.
pub fn compare_plans(
    expected: &::substrait::proto::Plan,
    actual: &::substrait::proto::Plan,
    config: &ProtoMatcherConfig,
) -> Vec<ProtoDifference> {
    // Convert both plans to JSON for comparison
    let expected_json = serde_json::to_value(expected).unwrap_or(Value::Null);
    let actual_json = serde_json::to_value(actual).unwrap_or(Value::Null);

    let mut differences = Vec::new();
    compare_json_values(&expected_json, &actual_json, "", config, &mut differences);

    differences
}

/// Recursively compare two JSON values and collect differences.
fn compare_json_values(
    expected: &Value,
    actual: &Value,
    path: &str,
    config: &ProtoMatcherConfig,
    differences: &mut Vec<ProtoDifference>,
) {
    // Stop if we've reached the max number of differences
    if differences.len() >= config.max_differences {
        return;
    }

    // Skip ignored fields
    if config.ignored_fields.contains(path) {
        return;
    }

    match (expected, actual) {
        (Value::Null, Value::Null) => {}
        (Value::Bool(e), Value::Bool(a)) if e == a => {}
        (Value::Number(e), Value::Number(a)) if e == a => {}
        (Value::String(e), Value::String(a)) if e == a => {}

        (Value::Array(e_arr), Value::Array(a_arr)) => {
            if e_arr.len() != a_arr.len() {
                differences.push(ProtoDifference::new(
                    format!("{}.length", path),
                    e_arr.len().to_string(),
                    a_arr.len().to_string(),
                ));
                return;
            }

            for (i, (e_val, a_val)) in e_arr.iter().zip(a_arr.iter()).enumerate() {
                let item_path = if path.is_empty() {
                    format!("[{}]", i)
                } else {
                    format!("{}[{}]", path, i)
                };
                compare_json_values(e_val, a_val, &item_path, config, differences);
            }
        }

        (Value::Object(e_obj), Value::Object(a_obj)) => {
            // Check for missing/extra keys
            let e_keys: HashSet<_> = e_obj.keys().collect();
            let a_keys: HashSet<_> = a_obj.keys().collect();

            for key in e_keys.difference(&a_keys) {
                let field_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{}.{}", path, key)
                };

                if !config.ignored_fields.contains(&field_path) {
                    differences.push(ProtoDifference::new(
                        field_path,
                        format!("{}", e_obj[*key]),
                        "missing".to_string(),
                    ));
                }
            }

            for key in a_keys.difference(&e_keys) {
                let field_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{}.{}", path, key)
                };

                if !config.ignored_fields.contains(&field_path) {
                    differences.push(ProtoDifference::new(
                        field_path,
                        "missing".to_string(),
                        format!("{}", a_obj[*key]),
                    ));
                }
            }

            // Compare common keys
            for key in e_keys.intersection(&a_keys) {
                let field_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{}.{}", path, key)
                };

                compare_json_values(&e_obj[*key], &a_obj[*key], &field_path, config, differences);
            }
        }

        // Different types or different values
        _ => {
            if !config.ignored_fields.contains(path) {
                differences.push(ProtoDifference::new(
                    path.to_string(),
                    format!("{}", expected),
                    format!("{}", actual),
                ));
            }
        }
    }
}

/// Format differences for display in test output.
pub fn format_differences(differences: &[ProtoDifference], max_display: usize) -> String {
    let mut output = String::new();

    if differences.is_empty() {
        output.push_str("No differences found.\n");
        return output;
    }

    output.push_str(&format!("Found {} difference(s):\n\n", differences.len()));

    for (i, diff) in differences.iter().take(max_display).enumerate() {
        output.push_str(&format!("{}. Path: {}\n", i + 1, diff.path));
        output.push_str(&format!("   Expected: {}\n", diff.expected));
        output.push_str(&format!("   Actual:   {}\n", diff.actual));
        output.push('\n');
    }

    if differences.len() > max_display {
        output.push_str(&format!(
            "... and {} more difference(s) not shown\n",
            differences.len() - max_display
        ));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_plans() {
        let plan = ::substrait::proto::Plan::default();
        let config = ProtoMatcherConfig::default();
        let diffs = compare_plans(&plan, &plan, &config);
        assert_eq!(diffs.len(), 0);
    }

    #[test]
    fn test_different_versions() {
        let mut plan1 = ::substrait::proto::Plan::default();
        let mut plan2 = ::substrait::proto::Plan::default();

        plan1.version = Some(::substrait::proto::Version {
            minor_number: 1,
            producer: "test1".to_string(),
            ..Default::default()
        });

        plan2.version = Some(::substrait::proto::Version {
            minor_number: 2,
            producer: "test2".to_string(),
            ..Default::default()
        });

        // Without ignoring version
        let config = ProtoMatcherConfig::default();
        let diffs = compare_plans(&plan1, &plan2, &config);
        assert!(diffs.len() > 0);

        // With ignoring version
        let config = ProtoMatcherConfig::ignoring_version();
        let diffs = compare_plans(&plan1, &plan2, &config);
        assert_eq!(diffs.len(), 0);
    }
}
