# Substrait TextPlan - Rust Library

This is a Rust implementation of the Substrait TextPlan library, designed to be used both directly from Rust and from other languages through FFI bindings.

## Overview

The Substrait TextPlan library provides functionality for parsing, loading, and converting Substrait plans in both text and binary formats. This Rust implementation mirrors the C++ implementation in the main repository, with some adaptations to make it more idiomatic in Rust.

## Features

- Parse textplans into a symbol table representation
- Convert textplans to binary protobuf plans
- Convert binary protobuf plans to textplans
- FFI bindings for C/C++ and Python
- Uses the same ANTLR grammar as the C++ implementation

## Building

### Requirements

- **Rust** (latest stable) and **Cargo**
- **Protocol Buffer Compiler (protoc)**
  - macOS: `brew install protobuf`
  - Ubuntu/Debian: `apt-get install protobuf-compiler`
  - Windows: Download from [protobuf releases](https://github.com/protocolbuffers/protobuf/releases)
  - Verify installation: `protoc --version`
- **Java JRE** (for ANTLR grammar processing)
  - Verify installation: `java -version`
- **ANTLR4 JAR with Rust support** (see setup steps below)
- **rustfmt** (recommended): `rustup component add rustfmt`

### Initial Setup

**IMPORTANT:** These steps must be completed before your first build.

1. **Clone the repository and initialize submodules:**
   ```bash
   git clone <repo-url>
   cd substrait-textplan
   git submodule update --init --recursive
   ```

   This pulls the Substrait protobuf definitions needed for the build.

2. **Download the ANTLR4 JAR with Rust support:**
   ```bash
   mkdir -p build-tools
   curl -L -o build-tools/antlr4rust.jar https://github.com/rrevenantt/antlr4rust/releases/download/antlr4-4.8-2-Rust0.3.0-beta/antlr4-4.8-2-SNAPSHOT-complete.jar
   ```

   **Important:** You MUST use this specific JAR file with Rust support. The standard ANTLR4 JAR does NOT support generating Rust code!

3. **Generate ANTLR parser code:**
   ```bash
   GENERATE_ANTLR=true cargo build
   ```

   This generates the parser code from the ANTLR grammar files. You only need to do this once, or when the grammar files change.

### Build Steps

After completing the initial setup above, you can build normally:

```bash
cargo build
```

Build in release mode:

```bash
cargo build --release
```

Run tests:

```bash
cargo test
```

## Using from Rust

```rust
use substrait_textplan::textplan::parser::load_from_text;

fn main() {
    let text = r#"
        schema simple_schema {
            id i32;
            name string;
            price fp64;
        }

        source LOCAL_FILES simple_source {
            ITEMS = [
                {
                    URI_FILE: "data.csv"
                }
            ]
        }

        read RELATION simple_read {
            SOURCE simple_source;
            BASE_SCHEMA simple_schema;
        }

        filter RELATION filtered_data {
            BASE_SCHEMA simple_schema;
            FILTER greater_than(price, 100.0_fp64);
        }

        simple_read -> filtered_data

        ROOT {
            NAMES = [filtered_data]
        }
    "#;

    match load_from_text(text) {
        Ok(binary_plan) => {
            println!("Successfully parsed plan: {} bytes", binary_plan.len());
        }
        Err(e) => {
            eprintln!("Error parsing plan: {}", e);
        }
    }
}
```

## Using from C++

Include the header file:

```cpp
#include "substrait_textplan.h"
```

Use the TextPlan class:

```cpp
#include <iostream>
#include "substrait_textplan.h"

int main() {
    std::string text = R"(
        schema simple_schema {
            id i32;
            name string;
            price fp64;
        }

        read RELATION simple_read {
            SOURCE simple_source;
            BASE_SCHEMA simple_schema;
        }
    )";

    auto binary_plan = substrait::textplan::TextPlan::LoadFromText(text);
    if (binary_plan.empty()) {
        std::cerr << "Error parsing plan" << std::endl;
        return 1;
    }

    std::cout << "Successfully parsed plan: " << binary_plan.size() << " bytes" << std::endl;
    return 0;
}
```

## Using from Python

```python
import substrait_textplan

text = """
    schema simple_schema {
        id i32;
        name string;
        price fp64;
    }

    source LOCAL_FILES simple_source {
        ITEMS = [
            {
                URI_FILE: "data.csv"
            }
        ]
    }

    read RELATION simple_read {
        SOURCE simple_source;
        BASE_SCHEMA simple_schema;
    }
"""

binary_plan = substrait_textplan.load_from_text(text)
if binary_plan is None:
    print("Error parsing plan")
else:
    print(f"Successfully parsed plan: {len(binary_plan)} bytes")
```

## ANTLR Grammar

This implementation uses ANTLR4 for parsing, using the same ANTLR grammar as the C++ implementation. The grammar files are located in:

- `grammar/SubstraitPlanLexer.g4`
- `grammar/SubstraitPlanParser.g4`

The generated parser code is placed in `src/textplan/parser/antlr/` and is excluded from git (must be generated locally).

### Regenerating ANTLR4 Code

If you need to regenerate the ANTLR parser code (after modifying grammar files):

```bash
GENERATE_ANTLR=true cargo build
```

The build script will look for the ANTLR4 JAR in these locations (in order):
1. Path specified by `ANTLR_JAR` environment variable
2. `build-tools/antlr4rust.jar`

If you haven't already set this up, see the "Initial Setup" section above for complete instructions.

## Implementation Status

This is an implementation of the Substrait TextPlan library in Rust. The current status is:

- [x] Basic structure and FFI bindings
- [x] Symbol table implementation
- [x] Tree-sitter parser integration
- [ ] ANTLR4 parser integration (in progress)
  - [x] Setup for antlr-rust
  - [x] Grammar files from C++ implementation
  - [x] Build-time code generation setup
  - [ ] Visitor implementations
  - [ ] Full ANTLR4 parser integration
- [x] Protobuf integration
- [ ] Full roundtrip parsing and generation

## License

Apache-2.0