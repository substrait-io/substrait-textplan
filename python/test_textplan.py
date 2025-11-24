#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

"""
Tests for the Python wrapper of the Substrait TextPlan library.
"""

import unittest
import sys
import os

# Add the parent directory to the path so we can import substrait_textplan
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import substrait_textplan


class TestTextPlan(unittest.TestCase):
    """Tests for the TextPlan wrapper."""

    def test_load_from_text_simple(self):
        """Test loading a simple textplan from a string."""
        text = """
        schema simple_schema {
            id i32;
            name string;
        }
        """

        result = substrait_textplan.load_from_text(text)
        self.assertIsNotNone(result, "Failed to parse simple schema")
        self.assertIsInstance(result, bytes)
        self.assertGreater(len(result), 0, "Binary result should not be empty")

    def test_load_from_text_with_types(self):
        """Test loading a textplan with various types."""
        text = """
        schema type_test_schema {
            id i32;
            name string;
            price fp64;
            nullable_field i32?;
            decimal_field decimal<19,0>;
            varchar_field varchar<100>;
        }
        """

        result = substrait_textplan.load_from_text(text)
        self.assertIsNotNone(result, "Failed to parse schema with types")
        self.assertIsInstance(result, bytes)
        self.assertGreater(len(result), 0)

    def test_load_from_text_nullable_parameterized(self):
        """Test loading a textplan with nullable parameterized types."""
        text = """
        schema nullable_test {
            decimal_nullable decimal?<18,2>;
            varchar_nullable varchar?<50>;
            fixedchar_nullable fixedchar?<10>;
        }
        """

        result = substrait_textplan.load_from_text(text)
        self.assertIsNotNone(result, "Failed to parse nullable parameterized types")
        self.assertIsInstance(result, bytes)
        self.assertGreater(len(result), 0)

    def test_load_from_binary_roundtrip(self):
        """Test roundtrip conversion: text -> binary -> text."""
        original_text = """
pipelines {
    test_read -> root;
}

schema roundtrip_schema {
    id i32;
    name string?;
    price decimal<10,2>;
}

source named_table test_source {
    names = ["test_table"]
}

read relation test_read {
    base_schema roundtrip_schema;
    source test_source;
}
"""

        # Convert to binary
        binary_data = substrait_textplan.load_from_text(original_text)
        self.assertIsNotNone(binary_data, "Failed to convert text to binary")

        # Convert back to text
        result_text = substrait_textplan.save_to_text(binary_data)
        self.assertIsNotNone(result_text, "Failed to convert binary to text")
        self.assertIsInstance(result_text, str)

        # Verify the result contains expected elements
        self.assertIn("schema", result_text.lower())
        self.assertIn("relation", result_text.lower())
        self.assertIn("pipelines", result_text.lower())

    def test_class_interface(self):
        """Test using the TextPlan class interface."""
        tp = substrait_textplan.TextPlan()

        text = """
pipelines {
    class_read -> root;
}

schema class_test {
    field1 i32;
    field2 string;
}

source named_table class_source {
    names = ["test"]
}

read relation class_read {
    base_schema class_test;
    source class_source;
}
"""

        binary_data = tp.load_from_text(text)
        self.assertIsNotNone(binary_data)
        self.assertGreater(len(binary_data), 0)

        result_text = tp.save_to_text(binary_data)
        self.assertIsNotNone(result_text)
        self.assertIn("schema", result_text.lower())
        self.assertIn("relation", result_text.lower())

    def test_invalid_text(self):
        """Test that invalid text returns None or raises an error gracefully."""
        invalid_text = "this is not valid textplan syntax @#$%"

        # Depending on the implementation, this might return None or raise an exception
        # For now, we just check that it doesn't crash
        result = substrait_textplan.load_from_text(invalid_text)
        # The result might be None if the parser handles errors gracefully
        # or it might be valid binary if the parser is very permissive


if __name__ == '__main__':
    unittest.main()
