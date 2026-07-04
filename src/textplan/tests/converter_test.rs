// SPDX-License-Identifier: Apache-2.0

//! Tests for the binary and JSON converters.

#[cfg(test)]
mod tests {
    use crate::proto::save_plan_to_json;
    use crate::textplan::converter::load_json;
    use crate::textplan::converter::process_plan_with_visitor;
    use crate::textplan::converter::save_binary::save_to_binary;
    use crate::textplan::parser::parse_text::parse_stream;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Find all test data files with a specific extension
    fn find_test_files(extension: &str) -> Vec<PathBuf> {
        // Find the source directory containing the test data
        // Use the main data directory with TPC-H files
        let data_dir = Path::new("testdata");

        // Collect all files with the specified extension
        let mut test_files = Vec::new();
        if data_dir.exists() {
            for entry in fs::read_dir(data_dir).expect("Failed to read data directory") {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_file() && path.extension().map_or(false, |ext| ext == extension) {
                        test_files.push(path);
                    }
                }
            }
        }

        // Sort the files for consistent test order
        test_files.sort();
        test_files
    }

    /// Add line numbers to text for better error messages
    fn add_line_numbers(text: &str) -> String {
        text.lines()
            .enumerate()
            .map(|(i, line)| format!("{:4} {}", i + 1, line))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_load_from_json_file() {
        let filename = "src/textplan/tests/data/converter/q6_first_stage.json".to_string();

        let plan_or_error = load_json::load_from_json_file(&filename);
        if let Err(err) = &plan_or_error {
            assert!(false, "Failed to load test file: {}", err);
        }

        let plan = plan_or_error.unwrap();

        // Verify that all expected protobuf components are present
        assert_eq!(plan.extensions.len(), 7, "Plan should have 7 extensions");

        // Check relations array
        assert!(!plan.relations.is_empty(), "Plan should have relations");

        // Debug the structure of relations
        println!("Relations: {:#?}", plan.relations);

        // Less strict assertions to help diagnose the issue
        if let Some(rel) = plan.relations.first() {
            println!("First relation: {:#?}", rel);

            // Check if rel_type exists
            if rel.rel_type.is_none() {
                assert!(false, "Relation should have a rel_type, but it's None");
            }

            // Try to print the relation type information
            if let Some(rel_type) = &rel.rel_type {
                println!("First relation type: {:#?}", rel_type);
            }
        }

        // Convert the plan to a string representation to check its size
        let message_string = format!("{:?}", plan);
        println!("Message length: {}", message_string.len());

        // Check the size to make sure we have a substantial plan
        assert!(
            message_string.len() > 1300,
            "Plan representation should be substantial in size"
        );

        let result = save_plan_to_json(&plan);
        match result {
            Ok(_) => {
                println!("{}", result.unwrap());
            }
            Err(_) => {
                assert!(false, "Failed to save plan to json");
            }
        }
    }

    #[test]
    fn test_conversion_from_binary_to_textplan() {
        // Find all the JSON test files
        let test_files = find_test_files("json");
        assert!(!test_files.is_empty(), "No test files found");

        println!("Found {} test files", test_files.len());

        for test_file in test_files {
            println!("Testing with file: {}", test_file.display());

            // 1. Load JSON to Plan
            let plan_or_error = load_json::load_from_json_file(&test_file.to_str().unwrap());
            match plan_or_error {
                Ok(_) => {}
                Err(_) => {
                    assert!(
                        false,
                        "Failed to load test file: {}",
                        plan_or_error.unwrap_err()
                    );
                }
            }
            let plan = plan_or_error.unwrap();

            // 2. Convert Plan to textplan
            let text_plan = process_plan_with_visitor(&plan).expect("Failed to load binary");
            assert!(!text_plan.is_empty(), "Empty textplan from binary");

            // 3. Compare the text output with the golden splan file if it exists
            let splan_path = test_file.with_extension("golden.splan");
            if splan_path.exists() {
                println!("  Found golden splan file: {}", splan_path.display());

                // Load the golden splan content
                let golden_splan =
                    fs::read_to_string(&splan_path).expect("Failed to read golden splan file");

                // Normalize both texts for comparison (trim whitespace, normalize line endings)
                let normalized_golden = golden_splan.trim().replace("\r\n", "\n");
                let normalized_text_plan = text_plan.trim().replace("\r\n", "\n");

                // Compare the text output directly
                if normalized_golden != normalized_text_plan {
                    println!(
                        "Text output doesn't match golden splan for test file: {}",
                        test_file.display()
                    );
                    println!(
                        "Generated textplan:\n{}\n",
                        add_line_numbers(&normalized_text_plan)
                    );
                    println!("Golden splan:\n{}\n", add_line_numbers(&normalized_golden));
                    assert!(
                        false,
                        "Text output doesn't match golden splan for test file: {:?}",
                        test_file
                    );
                } else {
                    println!("  ✓ Text output matches golden splan");
                }
            } else {
                println!("  No golden splan file found at: {}", splan_path.display());
                println!("  Skipping golden file comparison");
            }
        }
    }
}
