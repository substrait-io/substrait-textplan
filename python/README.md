# Substrait TextPlan Python Wrapper

Python bindings for the Substrait TextPlan library. Convert between Substrait TextPlan format and binary protobuf format.

## Installation

### From PyPI (recommended)

```bash
pip install substrait-textplan
```

The wheel includes pre-compiled binaries for Linux, macOS, and Windows.

### From Source

If you need to build from source, you'll need:
- Python 3.8+
- Rust toolchain
- Java JRE (for ANTLR grammar generation)

```bash
pip install substrait-textplan --no-binary substrait-textplan
```

Or clone and build:

```bash
git clone https://github.com/substrait-io/substrait-textplan.git
cd substrait-textplan/python
pip install -e .
```

## Quick Start

```python
import substrait_textplan

# Define a textplan
textplan = """
schema simple_schema {
    id i32;
    name string;
}

source named_table my_source {
    names = ["my_table"]
}

read relation my_read {
    base_schema simple_schema;
    source my_source;
}

pipelines {
    my_read -> root;
}
"""

# Convert to binary protobuf
binary = substrait_textplan.load_from_text(textplan)
print(f"Binary plan: {len(binary)} bytes")

# Convert back to text
result = substrait_textplan.save_to_text(binary)
print(result)
```

## API Reference

### Functions

#### `load_from_text(text: str) -> Optional[bytes]`

Load a textplan from a string and convert it to binary protobuf format.

**Parameters:**
- `text`: The textplan string to parse

**Returns:**
- Binary protobuf representation, or `None` if parsing failed

#### `save_to_text(data: bytes) -> Optional[str]`

Convert a binary protobuf plan to textplan format.

**Parameters:**
- `data`: Binary protobuf data

**Returns:**
- Textplan string representation, or `None` if conversion failed

### Class Interface

For more control, use the `TextPlan` class:

```python
from substrait_textplan import TextPlan

tp = TextPlan()
binary = tp.load_from_text(textplan_string)
text = tp.save_to_text(binary_data)
```

## Running Tests

```bash
python -m unittest discover -v
```

## License

Apache-2.0

## Links

- [GitHub Repository](https://github.com/substrait-io/substrait-textplan)
- [Issue Tracker](https://github.com/substrait-io/substrait-textplan/issues)
- [Substrait](https://substrait.io)
