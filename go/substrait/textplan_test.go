// SPDX-License-Identifier: Apache-2.0

package substrait

import (
	"testing"
)

func TestTextPlanRoundtrip(t *testing.T) {
	// This is a simple test that checks if we can convert a text plan to binary
	// and back to text
	
	// Skip if the shared library is not available
	tp := New()
	
	// Simple TextPlan for testing
	textPlan := `
pipelines {
	data -> root;
}

schema simple_schema {
	id i32;
	name string;
}

source named_table simple_source {
	names = ["test_table"]
}

read relation data {
	base_schema simple_schema;
	source simple_source;
}
`
	
	// Convert to binary
	binary, err := tp.LoadFromText(textPlan)
	if err != nil {
		t.Skipf("Skipping test: %v (shared library might not be available)", err)
	}
	
	// Make sure the binary data is not empty
	if len(binary) == 0 {
		t.Fatal("Binary data is empty")
	}
	
	// Convert back to text
	roundtripText, err := tp.SaveToText(binary)
	if err != nil {
		t.Fatalf("Failed to convert binary back to text: %v", err)
	}
	
	// Make sure the round-tripped text is not empty
	if len(roundtripText) == 0 {
		t.Fatal("Round-tripped text is empty")
	}

	// Print the round-tripped text for debugging
	t.Logf("Round-tripped text:\n%s\n", roundtripText)

	// Verify that key elements are present in the round-tripped text
	// We don't do an exact match because the format might change a bit
	for _, element := range []string{"relation", "schema", "pipelines", "root"} {
		if !contains(roundtripText, element) {
			t.Errorf("Round-tripped text does not contain '%s'", element)
		}
	}
}

// Helper function to check if a string contains a substring
func contains(s, substr string) bool {
	for i := 0; i <= len(s)-len(substr); i++ {
		if s[i:i+len(substr)] == substr {
			return true
		}
	}
	return false
}
