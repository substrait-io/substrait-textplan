// SPDX-License-Identifier: Apache-2.0

//! Roundtrip tests: JSON → Binary → Text → Binary → JSON
//!
//! These tests verify that we can convert a plan from JSON to binary,
//! then to text, parse it back, convert to binary again, and get the same result.
//! This follows the pattern from the C++ implementation in RoundtripTest.cpp.

#[cfg(test)]
mod tests {
    use crate::textplan::converter::load_json;
    use crate::textplan::converter::process_plan_with_visitor;
    use crate::textplan::converter::save_binary::save_to_binary;
    use crate::textplan::parser::parse_text::parse_stream;
    use crate::textplan::tests::proto_matchers::{
        compare_plans, format_differences, ProtoMatcherConfig,
    };

    /// Add line numbers to text for better error reporting
    fn add_line_numbers(text: &str) -> String {
        text.lines()
            .enumerate()
            .map(|(i, line)| format!("{:4}: {}", i + 1, line))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Normalize a plan for comparison, following C++ ReferenceNormalizer approach:
    /// 1. Sort extension_uris by URI string and renumber anchors from 1
    /// 2. Sort extensions by (uri_reference, name) and renumber function_anchor from 0
    /// 3. Update all references throughout the plan
    fn normalize_plan(mut plan: ::substrait::proto::Plan) -> ::substrait::proto::Plan {
        use std::collections::HashMap;

        // Clear version field (always ignored in comparison)
        plan.version = None;

        // Step 1: Normalize extension URI spaces
        let mut uri_mapping: HashMap<u32, u32> = HashMap::new();

        // Sort by URI string
        plan.extension_uris.sort_by(|a, b| a.uri.cmp(&b.uri));

        // Renumber from 1 and build mapping
        for (new_anchor, uri) in plan.extension_uris.iter_mut().enumerate() {
            let old_anchor = uri.extension_uri_anchor;
            let new_anchor_val = (new_anchor + 1) as u32;
            uri_mapping.insert(old_anchor, new_anchor_val);
            uri.extension_uri_anchor = new_anchor_val;
        }

        // Update function URI references
        for ext in plan.extensions.iter_mut() {
            if let Some(::substrait::proto::extensions::simple_extension_declaration::MappingType::ExtensionFunction(ref mut f)) = ext.mapping_type {
                if let Some(&new_ref) = uri_mapping.get(&f.extension_uri_reference) {
                    f.extension_uri_reference = new_ref;
                }
            }
        }

        // Step 2: Normalize function references
        let mut function_mapping: HashMap<u32, u32> = HashMap::new();

        // Sort by (uri_reference, name)
        plan.extensions.sort_by(|a, b| {
            let a_func = if let Some(::substrait::proto::extensions::simple_extension_declaration::MappingType::ExtensionFunction(ref f)) = a.mapping_type {
                Some(f)
            } else {
                None
            };
            let b_func = if let Some(::substrait::proto::extensions::simple_extension_declaration::MappingType::ExtensionFunction(ref f)) = b.mapping_type {
                Some(f)
            } else {
                None
            };
            match (a_func, b_func) {
                (Some(af), Some(bf)) => {
                    (af.extension_uri_reference, &af.name).cmp(&(bf.extension_uri_reference, &bf.name))
                }
                _ => std::cmp::Ordering::Equal,
            }
        });

        // Renumber from 0 and build mapping
        for (new_anchor, ext) in plan.extensions.iter_mut().enumerate() {
            if let Some(::substrait::proto::extensions::simple_extension_declaration::MappingType::ExtensionFunction(ref mut f)) = ext.mapping_type {
                let old_anchor = f.function_anchor;
                let new_anchor_val = new_anchor as u32;
                function_mapping.insert(old_anchor, new_anchor_val);
                f.function_anchor = new_anchor_val;
            }
        }

        // Step 3: Update all function references in relations (recursively)
        for relation in plan.relations.iter_mut() {
            normalize_plan_relation(relation, &function_mapping);
        }

        plan
    }

    /// Recursively normalize function references in expressions
    fn normalize_expression(
        expr: &mut ::substrait::proto::Expression,
        mapping: &std::collections::HashMap<u32, u32>,
    ) {
        use ::substrait::proto::expression::RexType;

        match &mut expr.rex_type {
            Some(RexType::ScalarFunction(ref mut func)) => {
                if let Some(&new_ref) = mapping.get(&func.function_reference) {
                    func.function_reference = new_ref;
                }
                for arg in func.arguments.iter_mut() {
                    if let Some(::substrait::proto::function_argument::ArgType::Value(
                        ref mut val,
                    )) = arg.arg_type
                    {
                        normalize_expression(val, mapping);
                    }
                }
            }
            Some(RexType::Cast(ref mut cast)) => {
                if let Some(ref mut input) = cast.input {
                    normalize_expression(input, mapping);
                }
            }
            Some(RexType::IfThen(ref mut if_then)) => {
                for if_clause in if_then.ifs.iter_mut() {
                    if let Some(ref mut if_expr) = if_clause.r#if {
                        normalize_expression(if_expr, mapping);
                    }
                    if let Some(ref mut then_expr) = if_clause.then {
                        normalize_expression(then_expr, mapping);
                    }
                }
                if let Some(ref mut else_expr) = if_then.r#else {
                    normalize_expression(else_expr, mapping);
                }
            }
            Some(RexType::Subquery(ref mut subquery)) => {
                use ::substrait::proto::expression::subquery::SubqueryType;
                match &mut subquery.subquery_type {
                    Some(SubqueryType::Scalar(ref mut scalar)) => {
                        if let Some(ref mut input) = scalar.input {
                            normalize_relation(input, mapping);
                        }
                    }
                    Some(SubqueryType::InPredicate(ref mut in_pred)) => {
                        for needle in in_pred.needles.iter_mut() {
                            normalize_expression(needle, mapping);
                        }
                        if let Some(ref mut haystack) = in_pred.haystack {
                            normalize_relation(haystack, mapping);
                        }
                    }
                    Some(SubqueryType::SetPredicate(ref mut set_pred)) => {
                        if let Some(ref mut tuples) = set_pred.tuples {
                            normalize_relation(tuples, mapping);
                        }
                    }
                    Some(SubqueryType::SetComparison(ref mut set_comp)) => {
                        if let Some(ref mut left) = set_comp.left {
                            normalize_expression(left, mapping);
                        }
                        if let Some(ref mut right) = set_comp.right {
                            normalize_relation(right, mapping);
                        }
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    /// Recursively normalize function references in relations
    fn normalize_relation(
        rel: &mut ::substrait::proto::Rel,
        mapping: &std::collections::HashMap<u32, u32>,
    ) {
        use ::substrait::proto::rel::RelType;

        match &mut rel.rel_type {
            Some(RelType::Read(ref mut read)) => {
                if let Some(ref mut filter) = read.filter {
                    normalize_expression(filter, mapping);
                }
                if let Some(ref mut best_effort_filter) = read.best_effort_filter {
                    normalize_expression(best_effort_filter, mapping);
                }
            }
            Some(RelType::Filter(ref mut filter)) => {
                if let Some(ref mut input) = filter.input {
                    normalize_relation(input, mapping);
                }
                if let Some(ref mut condition) = filter.condition {
                    normalize_expression(condition, mapping);
                }
            }
            Some(RelType::Fetch(ref mut fetch)) => {
                if let Some(ref mut input) = fetch.input {
                    normalize_relation(input, mapping);
                }
            }
            Some(RelType::Aggregate(ref mut agg)) => {
                if let Some(ref mut input) = agg.input {
                    normalize_relation(input, mapping);
                }
                for measure in agg.measures.iter_mut() {
                    if let Some(ref mut agg_func) = measure.measure {
                        if let Some(&new_ref) = mapping.get(&agg_func.function_reference) {
                            agg_func.function_reference = new_ref;
                        }
                        for arg in agg_func.arguments.iter_mut() {
                            if let Some(::substrait::proto::function_argument::ArgType::Value(
                                ref mut val,
                            )) = arg.arg_type
                            {
                                normalize_expression(val, mapping);
                            }
                        }
                    }
                }
            }
            Some(RelType::Sort(ref mut sort)) => {
                if let Some(ref mut input) = sort.input {
                    normalize_relation(input, mapping);
                }
                for sort_field in sort.sorts.iter_mut() {
                    if let Some(ref mut expr) = sort_field.expr {
                        normalize_expression(expr, mapping);
                    }
                }
            }
            Some(RelType::Join(ref mut join)) => {
                if let Some(ref mut left) = join.left {
                    normalize_relation(left, mapping);
                }
                if let Some(ref mut right) = join.right {
                    normalize_relation(right, mapping);
                }
                if let Some(ref mut expression) = join.expression {
                    normalize_expression(expression, mapping);
                }
                if let Some(ref mut post_join_filter) = join.post_join_filter {
                    normalize_expression(post_join_filter, mapping);
                }
            }
            Some(RelType::Project(ref mut project)) => {
                if let Some(ref mut input) = project.input {
                    normalize_relation(input, mapping);
                }
                for expr in project.expressions.iter_mut() {
                    normalize_expression(expr, mapping);
                }
            }
            Some(RelType::Set(ref mut set)) => {
                for input in set.inputs.iter_mut() {
                    normalize_relation(input, mapping);
                }
            }
            Some(RelType::ExtensionSingle(ref mut ext)) => {
                if let Some(ref mut input) = ext.input {
                    normalize_relation(input, mapping);
                }
            }
            Some(RelType::ExtensionMulti(ref mut ext)) => {
                for input in ext.inputs.iter_mut() {
                    normalize_relation(input, mapping);
                }
            }
            Some(RelType::Cross(ref mut cross)) => {
                if let Some(ref mut left) = cross.left {
                    normalize_relation(left, mapping);
                }
                if let Some(ref mut right) = cross.right {
                    normalize_relation(right, mapping);
                }
            }
            Some(RelType::HashJoin(ref mut hash_join)) => {
                if let Some(ref mut left) = hash_join.left {
                    normalize_relation(left, mapping);
                }
                if let Some(ref mut right) = hash_join.right {
                    normalize_relation(right, mapping);
                }
                if let Some(ref mut post_join_filter) = hash_join.post_join_filter {
                    normalize_expression(post_join_filter, mapping);
                }
            }
            Some(RelType::MergeJoin(ref mut merge_join)) => {
                if let Some(ref mut left) = merge_join.left {
                    normalize_relation(left, mapping);
                }
                if let Some(ref mut right) = merge_join.right {
                    normalize_relation(right, mapping);
                }
                if let Some(ref mut post_join_filter) = merge_join.post_join_filter {
                    normalize_expression(post_join_filter, mapping);
                }
            }
            _ => {}
        }
    }

    /// Normalize function references in a PlanRel (root or rel)
    fn normalize_plan_relation(
        plan_rel: &mut ::substrait::proto::PlanRel,
        mapping: &std::collections::HashMap<u32, u32>,
    ) {
        use ::substrait::proto::plan_rel::RelType;

        match &mut plan_rel.rel_type {
            Some(RelType::Root(ref mut root)) => {
                if let Some(ref mut input) = root.input {
                    normalize_relation(input, mapping);
                }
            }
            Some(RelType::Rel(ref mut rel)) => {
                normalize_relation(rel, mapping);
            }
            None => {}
        }
    }

    /// Helper function for roundtrip tests: JSON → Plan → TextPlan → SymbolTable → Binary → Plan comparison.
    /// Like the C++ implementation, we ignore the version field when comparing plans.
    fn run_roundtrip_test(test_file: &str) {
        let file_path = format!("testdata/{}", test_file);
        println!("\n=== Roundtrip test for: {} ===", test_file);

        // Step 1: Load JSON → Plan
        let original_plan = match load_json::load_from_json_file(&file_path) {
            Ok(plan) => plan,
            Err(err) => {
                panic!("Failed to load test file {}: {}", file_path, err);
            }
        };

        // Step 2: Plan → Binary (to verify encoding works)
        let original_binary = match crate::proto::save_plan_to_binary(&original_plan) {
            Ok(binary) => binary,
            Err(err) => {
                panic!(
                    "Failed to serialize original plan to binary for {}: {}",
                    file_path, err
                );
            }
        };

        println!("Original binary size: {} bytes", original_binary.len());

        // Step 3: Binary → Plan (verify we can deserialize what we just serialized)
        let loaded_plan = match crate::proto::load_plan_from_binary(&original_binary) {
            Ok(plan) => plan,
            Err(err) => {
                panic!(
                    "Failed to deserialize original binary for {}: {}",
                    file_path, err
                );
            }
        };

        // Step 4: Plan → TextPlan
        let text_plan = match process_plan_with_visitor(&loaded_plan) {
            Ok(text) => text,
            Err(err) => {
                panic!(
                    "Failed to convert binary to text for {}: {}",
                    file_path, err
                );
            }
        };

        assert!(!text_plan.is_empty(), "Empty textplan from binary");
        println!("Generated textplan ({} bytes)", text_plan.len());
        println!(
            "\n=== Generated TextPlan ===\n{}",
            add_line_numbers(&text_plan)
        );

        // Step 5: TextPlan → Parse → Symbol Table
        let parse_result = parse_stream(&text_plan);

        if !parse_result.successful() {
            println!("Generated textplan that failed to parse:");
            println!("{}", add_line_numbers(&text_plan));
            panic!(
                "Failed to parse generated textplan for {}: {:?}",
                file_path,
                parse_result.all_errors()
            );
        }

        let symbol_table = parse_result.symbol_table();
        println!(
            "Parsed successfully, symbol table has {} symbols",
            symbol_table.len()
        );

        // Step 6: Symbol Table → Binary
        let roundtrip_binary = match save_to_binary(&symbol_table) {
            Ok(binary) => binary,
            Err(err) => {
                panic!(
                    "Failed to convert symbol table to binary for {}: {}",
                    file_path, err
                );
            }
        };

        println!("Roundtrip binary size: {} bytes", roundtrip_binary.len());

        // Step 7: Binary → Plan (deserialize roundtrip result)
        let roundtrip_plan = match crate::proto::load_plan_from_binary(&roundtrip_binary) {
            Ok(plan) => plan,
            Err(err) => {
                panic!(
                    "Failed to deserialize roundtrip binary for {}: {}",
                    file_path, err
                );
            }
        };

        // Step 8: Normalize both plans for comparison (like C++ ReferenceNormalizer)
        let normalized_original = normalize_plan(original_plan.clone());
        let normalized_roundtrip = normalize_plan(roundtrip_plan.clone());

        // Step 9: Compare normalized plans using proto matchers
        let config = ProtoMatcherConfig::ignoring_version();
        let differences = compare_plans(&normalized_original, &normalized_roundtrip, &config);

        if !differences.is_empty() {
            // Plans differ - show detailed differences and intermediate textplan
            let original_json = crate::proto::save_plan_to_json(&original_plan)
                .unwrap_or_else(|_| "Failed to serialize original plan".to_string());
            let roundtrip_json = crate::proto::save_plan_to_json(&roundtrip_plan)
                .unwrap_or_else(|_| "Failed to serialize roundtrip plan".to_string());

            eprintln!("\n{}", "=".repeat(80));
            eprintln!("ROUNDTRIP TEST FAILED: {}", file_path);
            eprintln!("{}", "=".repeat(80));

            eprintln!("\n{}", format_differences(&differences, 10));

            eprintln!("{}", "=".repeat(80));
            eprintln!("INTERMEDIATE TEXTPLAN:");
            eprintln!("{}", "=".repeat(80));
            eprintln!("{}", add_line_numbers(&text_plan));

            eprintln!("\n{}", "=".repeat(80));
            eprintln!("ORIGINAL PLAN JSON:");
            eprintln!("{}", "=".repeat(80));
            eprintln!("{}", original_json);

            eprintln!("\n{}", "=".repeat(80));
            eprintln!("ROUNDTRIP PLAN JSON:");
            eprintln!("{}", "=".repeat(80));
            eprintln!("{}", roundtrip_json);

            panic!(
                "Roundtrip plan does not match original for {}\n\
                 Found {} difference(s)\n\
                 Original: {} bytes binary, {} bytes JSON\n\
                 Roundtrip: {} bytes binary, {} bytes JSON\n\
                 See detailed output above for differences and intermediate textplan",
                file_path,
                differences.len(),
                original_binary.len(),
                original_json.len(),
                roundtrip_binary.len(),
                roundtrip_json.len()
            );
        }

        println!("✓ Roundtrip successful: Plans match for {}", file_path);
    }

    // Macro to generate individual test functions for each data file
    macro_rules! roundtrip_tests {
        ($($name:ident: $file:expr,)*) => {
            $(
                #[test]
                fn $name() {
                    run_roundtrip_test($file);
                }
            )*
        }
    }

    // Generate a test for each JSON file in the test data directory
    roundtrip_tests! {
        test_roundtrip_set_comparison_any: "set-comparision-any.json",
        test_roundtrip_tpch_plan01: "tpch-plan01.json",
        test_roundtrip_tpch_plan02: "tpch-plan02.json",
        test_roundtrip_tpch_plan03: "tpch-plan03.json",
        test_roundtrip_tpch_plan04: "tpch-plan04.json",
        test_roundtrip_tpch_plan05: "tpch-plan05.json",
        test_roundtrip_tpch_plan06: "tpch-plan06.json",
        test_roundtrip_tpch_plan07: "tpch-plan07.json",
        test_roundtrip_tpch_plan09: "tpch-plan09.json",
        test_roundtrip_tpch_plan10: "tpch-plan10.json",
        test_roundtrip_tpch_plan11: "tpch-plan11.json",
        test_roundtrip_tpch_plan13: "tpch-plan13.json",
        test_roundtrip_tpch_plan14: "tpch-plan14.json",
        test_roundtrip_tpch_plan16: "tpch-plan16.json",
        test_roundtrip_tpch_plan17: "tpch-plan17.json",
        test_roundtrip_tpch_plan18: "tpch-plan18.json",
        test_roundtrip_tpch_plan19: "tpch-plan19.json",
        test_roundtrip_tpch_plan20: "tpch-plan20.json",
        test_roundtrip_tpch_plan21: "tpch-plan21.json",
        test_roundtrip_tpch_plan22: "tpch-plan22.json",
    }
}
