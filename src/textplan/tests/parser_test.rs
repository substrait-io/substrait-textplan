// SPDX-License-Identifier: Apache-2.0

//! Tests for the parser and printer.
//!
//! These tests are based on the C++ TextPlanParserTest.cpp tests to ensure
//! compatibility between the C++ and Rust implementations.

#[cfg(test)]
mod tests {
    use crate::textplan::parser::parse_stream;

    struct TestCase {
        name: &'static str,
        input: &'static str,
        expected_symbols: Vec<&'static str>,
        should_succeed: bool,
    }

    fn get_test_cases() -> Vec<TestCase> {
        vec![
            TestCase {
                name: "test1-unused-extension-space",
                input: "extension_space blah.yaml {}",
                expected_symbols: vec!["blah.yaml"],
                should_succeed: true,
            },
            TestCase {
                name: "test1-used-extension-space",
                input: "extension_space blah.yaml { function concat:str as concat; }",
                expected_symbols: vec!["blah.yaml", "concat"],
                should_succeed: true,
            },
            TestCase {
                name: "test2-pipelines-no-relations",
                input: r"pipelines {
                    root -> project -> read;
                }",
                expected_symbols: vec!["read", "project", "root"],
                should_succeed: true,
            },
            TestCase {
                name: "test3-schema",
                input: r"schema schema {
                    r_regionkey i32;
                    r_name string?;
                    r_comment string;
                }",
                expected_symbols: vec!["schema", "r_regionkey", "r_name", "r_comment"],
                should_succeed: true,
            },
            TestCase {
                name: "test4-source",
                input: r##"source named_table named {
                    names = [
                        "#2",
                    ]
                }"##,
                expected_symbols: vec!["named", "#2"],
                should_succeed: true,
            },
            TestCase {
                name: "test6-read-relation",
                input: r"read relation myread {
                    base_schema schemaone;
                    source mynamedtable;
                }",
                expected_symbols: vec!["myread"],
                should_succeed: true,
            },
            TestCase {
                name: "test6b-capital-read-relation",
                input: r"READ relation myread {
                    base_schema schemaone;
                    source mynamedtable;
                }",
                expected_symbols: vec!["myread"],
                should_succeed: true,
            },
            TestCase {
                name: "test-simple-plan",
                input: r##"
                schema schema {
                    r_regionkey i32;
                    r_name string?;
                    r_comment string;
                }

                source named_table named {
                    names = [
                        "#2",
                    ]
                }

                read relation myread {
                    base_schema schema;
                    source named;
                }
                "##,
                expected_symbols: vec![
                    "schema",
                    "r_regionkey",
                    "r_name",
                    "r_comment",
                    "named",
                    "#2",
                    "myread",
                ],
                should_succeed: true,
            },
            TestCase {
                name: "test-empty-plan",
                input: "",
                expected_symbols: vec![],
                should_succeed: true,
            },
            TestCase {
                name: "test-no-leading-whitespace",
                input: r##"schema schema {
  r_regionkey i32;
}
"##,
                expected_symbols: vec!["schema", "r_regionkey"],
                should_succeed: true,
            },
        ]
    }

    #[test]
    fn test_parser_cases() {
        for test_case in get_test_cases() {
            println!("\n=== Running test: {} ===", test_case.name);
            let result = parse_stream(test_case.input);

            if test_case.should_succeed {
                assert!(
                    result.successful(),
                    "Test '{}' failed: {:?}",
                    test_case.name,
                    result.all_errors()
                );

                let symbol_table = result.symbol_table();
                let symbols = symbol_table.symbols();

                // Check that all expected symbols are present
                for expected_symbol in &test_case.expected_symbols {
                    let found = symbols.iter().any(|s| s.name() == *expected_symbol);
                    assert!(
                        found,
                        "Test '{}': Expected symbol '{}' not found in symbol table. Found symbols: {:?}",
                        test_case.name,
                        expected_symbol,
                        symbols.iter().map(|s| s.name()).collect::<Vec<_>>()
                    );
                }

                println!(
                    "Test '{}' passed with {} symbols",
                    test_case.name,
                    symbols.len()
                );
            } else {
                assert!(
                    !result.successful(),
                    "Test '{}' should have failed but succeeded",
                    test_case.name
                );
            }
        }
    }

    #[test]
    #[ignore] // TODO: antlr4rust has a bug with lexer modes - URI tokens in EXTENSIONS mode don't work
              // See: https://github.com/rrevenantt/antlr4rust - lexer mode transitions not properly handled
              // The C++ version uses the same grammar and works fine, but Rust fails to recognize URI tokens
              // after switching to EXTENSIONS mode. This needs to be fixed in antlr4rust or we need a workaround.
    fn test_parse_provided_sample() {
        let text =
            std::fs::read_to_string("src/substrait/textplan/parser/data/provided_sample1.splan")
                .expect("Failed to read provided_sample1.splan");
        let parse_result = parse_stream(&text);
        assert!(
            parse_result.successful(),
            "Parse failed: {:?}",
            parse_result.all_errors()
        );

        // Verify we have symbols
        let symbol_table = parse_result.symbol_table();
        assert!(symbol_table.len() > 0, "Symbol table should not be empty");
    }

    /// Tests type parsing including nullable markers and parameterized types.
    /// Based on C++ TypeTest.cpp to ensure compatibility.
    #[test]
    fn test_type_parsing() {
        struct TypeTestCase {
            name: &'static str,
            type_text: &'static str,
            should_succeed: bool,
        }

        let test_cases = vec![
            // Basic types
            TypeTestCase {
                name: "boolean",
                type_text: "boolean",
                should_succeed: true,
            },
            TypeTestCase {
                name: "i32",
                type_text: "i32",
                should_succeed: true,
            },
            TypeTestCase {
                name: "string",
                type_text: "string",
                should_succeed: true,
            },
            // Nullable basic types
            TypeTestCase {
                name: "nullable-i32",
                type_text: "i32?",
                should_succeed: true,
            },
            TypeTestCase {
                name: "nullable-string",
                type_text: "string?",
                should_succeed: true,
            },
            // Parameterized types (non-nullable)
            TypeTestCase {
                name: "decimal",
                type_text: "decimal<19,0>",
                should_succeed: true,
            },
            TypeTestCase {
                name: "decimal-full-precision",
                type_text: "decimal<18,2>",
                should_succeed: true,
            },
            TypeTestCase {
                name: "varchar",
                type_text: "varchar<44>",
                should_succeed: true,
            },
            TypeTestCase {
                name: "fixedchar",
                type_text: "fixedchar<1>",
                should_succeed: true,
            },
            TypeTestCase {
                name: "fixedbinary",
                type_text: "fixedbinary<16>",
                should_succeed: true,
            },
            // Nullable parameterized types - ? before <
            TypeTestCase {
                name: "nullable-decimal",
                type_text: "decimal?<19,0>",
                should_succeed: true,
            },
            TypeTestCase {
                name: "nullable-decimal-precision",
                type_text: "decimal?<18,2>",
                should_succeed: true,
            },
            TypeTestCase {
                name: "nullable-varchar",
                type_text: "varchar?<44>",
                should_succeed: true,
            },
            TypeTestCase {
                name: "nullable-fixedchar",
                type_text: "fixedchar?<1>",
                should_succeed: true,
            },
            // Container types
            TypeTestCase {
                name: "list-string",
                type_text: "list<string>",
                should_succeed: true,
            },
            TypeTestCase {
                name: "list-nullable-string",
                type_text: "list<string?>",
                should_succeed: true,
            },
            TypeTestCase {
                name: "struct-simple",
                type_text: "struct<i32,string>",
                should_succeed: true,
            },
            TypeTestCase {
                name: "struct-with-nullable",
                type_text: "struct<i32,string?>",
                should_succeed: true,
            },
            TypeTestCase {
                name: "map-simple",
                type_text: "map<string,i32>",
                should_succeed: true,
            },
        ];

        for test_case in test_cases {
            println!(
                "\n=== Testing type: {} ({}) ===",
                test_case.name, test_case.type_text
            );

            // Create a schema with the type
            let input = format!(
                "schema test_schema {{\n    test_field {};\n}}",
                test_case.type_text
            );

            let result = parse_stream(&input);

            if test_case.should_succeed {
                assert!(
                    result.successful(),
                    "Type '{}' (text: '{}') failed to parse: {:?}",
                    test_case.name,
                    test_case.type_text,
                    result.all_errors()
                );

                println!("Type '{}' parsed successfully", test_case.name);
            } else {
                assert!(
                    !result.successful(),
                    "Type '{}' should have failed but succeeded",
                    test_case.name
                );
            }
        }
    }
}
