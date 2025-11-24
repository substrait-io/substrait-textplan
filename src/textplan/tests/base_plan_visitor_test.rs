// SPDX-License-Identifier: Apache-2.0

//! Tests for the BasePlanProtoVisitor.

#[cfg(test)]
mod tests {
    use crate::textplan::common::ProtoLocation;
    use crate::textplan::converter::load_json;
    use crate::textplan::converter::PlanProtoVisitor;
    use crate::textplan::converter::Traversable;

    // A simple implementation of PlanProtoVisitor for testing
    // This tracker keeps a trace of all the locations visited during traversal
    struct LocationTrackingVisitor {
        current_loc: ProtoLocation,
        visited_locations: Vec<String>,
    }

    impl LocationTrackingVisitor {
        fn new() -> Self {
            Self {
                current_loc: ProtoLocation::unknown(),
                visited_locations: Vec::new(),
            }
        }

        // Record current location during traversal
        fn record_location(&mut self) {
            let location_str = format!("{}", self.current_loc);
            self.visited_locations.push(location_str);
        }

        // Get the recorded locations in a vector
        fn get_locations(&self) -> &Vec<String> {
            &self.visited_locations
        }

        fn visit_plan(&mut self, obj: &substrait::proto::Plan) {
            obj.traverse(self);
        }
    }

    impl PlanProtoVisitor for LocationTrackingVisitor {
        fn current_location(&self) -> &ProtoLocation {
            &self.current_loc
        }

        fn set_location(&mut self, location: ProtoLocation) {
            self.current_loc = location;
            self.record_location();
        }
    }

    #[test]
    fn test_visitor_location_tracking() {
        // Load a real plan from the test data
        let filename = "src/textplan/tests/data/converter/q6_first_stage.json";
        let plan_or_error = load_json::load_from_json_file(filename);

        // Make sure we loaded the plan successfully
        assert!(
            plan_or_error.is_ok(),
            "Failed to load test plan: {:?}",
            plan_or_error.err()
        );
        let plan = plan_or_error.unwrap();

        // Create a visitor to track locations
        let mut visitor = LocationTrackingVisitor::new();

        // Visit the plan
        visitor.visit_plan(&plan);

        // Get the recorded locations
        let locations = visitor.get_locations();

        // Ensure locations were recorded
        assert!(
            !locations.is_empty(),
            "Should have recorded locations during traversal"
        );

        // The first location should be the plan object itself
        assert!(
            locations[0].starts_with("proto type"),
            "First location should be the plan object"
        );

        // Print locations for debugging
        for (i, loc) in locations.iter().enumerate() {
            println!("{}: {}", i, loc);
        }

        // Check that we have paths with certain keywords
        let has_extensions = locations.iter().any(|loc| loc.contains("extensions"));
        assert!(has_extensions, "Should have visited extensions");

        let has_relations = locations.iter().any(|loc| loc.contains("relations"));
        assert!(has_relations, "Should have visited relations");

        // Find paths that verify the location tracking is working
        let has_deep_path = locations
            .iter()
            .any(|loc| loc.contains(".") && loc.matches('.').count() >= 2);
        assert!(has_deep_path, "Should have visited at least one deep path");
    }

    #[test]
    fn test_location_path_building() {
        // Load a real plan from the test data
        let filename = "src/textplan/tests/data/converter/q6_first_stage.json";
        let plan_or_error = load_json::load_from_json_file(filename);
        assert!(plan_or_error.is_ok(), "Failed to load test plan");
        let plan = plan_or_error.unwrap();

        // Create a visitor
        let mut visitor = LocationTrackingVisitor::new();

        // Visit the plan
        visitor.visit_plan(&plan);

        // Get the recorded locations
        let locations = visitor.get_locations();

        // The first location should be the plan itself
        assert!(!locations.is_empty(), "Should have at least one location");
        let plan_location = &locations[0];
        assert!(plan_location.starts_with("proto type"));
        assert!(!plan_location.contains("."));

        // Print all locations for debugging
        for (i, loc) in locations.iter().enumerate() {
            println!("{}: {}", i, loc);
        }

        // Verify path building with nested paths

        // Find deep paths to verify full path construction
        let deep_paths = locations
            .iter()
            .filter(|loc| loc.matches('.').count() >= 3)
            .collect::<Vec<_>>();

        // We should have some deep paths
        assert!(
            !deep_paths.is_empty(),
            "Should have some deeply nested paths"
        );

        // Find paths that follow a hierarchy
        let relations_paths = locations
            .iter()
            .filter(|loc| loc.contains("relations"))
            .collect::<Vec<_>>();

        // We should have some relation paths
        assert!(!relations_paths.is_empty(), "Should have visited relations");

        // At least one of these paths should be deeply nested
        let has_nested_relation_path = relations_paths
            .iter()
            .any(|loc| loc.matches('.').count() >= 2);

        assert!(
            has_nested_relation_path,
            "Should have at least one deeply nested relation path"
        );

        // Verify that paths follow correct nesting pattern (parent path is substring of child path)
        let mut has_proper_nesting = false;
        for (i, path1) in locations.iter().enumerate() {
            if !path1.contains(".") {
                continue;
            }

            for path2 in &locations[i + 1..] {
                if path2.starts_with(path1) && path2.len() > path1.len() {
                    // Found a child path that extends a parent path
                    has_proper_nesting = true;
                    break;
                }
            }

            if has_proper_nesting {
                break;
            }
        }

        assert!(
            has_proper_nesting,
            "Should have proper path nesting pattern"
        );
    }

    #[test]
    fn test_location_restoration() {
        // Load a real plan from the test data
        let filename = "src/textplan/tests/data/converter/q6_first_stage.json";
        let plan_or_error = load_json::load_from_json_file(filename);
        assert!(plan_or_error.is_ok(), "Failed to load test plan");
        let plan = plan_or_error.unwrap();

        // Create a visitor
        let mut visitor = LocationTrackingVisitor::new();

        // Visit the plan
        visitor.visit_plan(&plan);

        // Get the recorded locations
        let locations = visitor.get_locations();

        // Find deep paths
        let deep_indices: Vec<usize> = locations
            .iter()
            .enumerate()
            .filter(|(_, loc)| loc.matches('.').count() >= 3)
            .map(|(i, _)| i)
            .collect();

        // Ensure we have some deep paths
        assert!(
            !deep_indices.is_empty(),
            "Should have some deeply nested paths"
        );

        // For each deep path, verify that we eventually return to a less nested path
        for deep_idx in deep_indices {
            // Skip paths that are too close to the end
            if deep_idx >= locations.len() - 2 {
                continue;
            }

            let deep_path = &locations[deep_idx];
            let deep_dot_count = deep_path.matches('.').count();

            // Find if there's a later path with fewer dots (less nesting)
            let restoration_found = locations[deep_idx + 1..]
                .iter()
                .any(|loc| loc.matches('.').count() < deep_dot_count);

            assert!(
                restoration_found,
                "Location should eventually be restored after deep path: {}",
                deep_path
            );
        }
    }
}
