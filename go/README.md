# Substrait TextPlan Go Wrapper

This package provides Go bindings for the Substrait TextPlan library. It allows you to convert between Substrait TextPlan format and binary protobuf format directly from Go code.

## Prerequisites

This package uses CGO to interface with the Substrait TextPlan Rust library. You must have the following installed:

1. **Go 1.19 or later**
2. **Rust toolchain** (for building the native library)
3. **C compiler** (required by CGO)
4. **Java JRE** (for ANTLR grammar generation)

## Installation

### Step 1: Build the Rust Library

First, build the Substrait TextPlan Rust library:

```bash
# Clone the repository
git clone https://github.com/substrait-io/substrait-textplan.git
cd substrait-textplan

# Build the Rust library (this generates ANTLR parsers and builds the native library)
GENERATE_ANTLR=true cargo build --release
```

### Step 2: Install the Library

The shared library needs to be accessible to the Go runtime. Options:

**Option A: Set library path (recommended for development)**
```bash
# On Linux
export LD_LIBRARY_PATH=$PWD/target/release:$LD_LIBRARY_PATH

# On macOS
export DYLD_LIBRARY_PATH=$PWD/target/release:$DYLD_LIBRARY_PATH
```

**Option B: Install system-wide**
```bash
# On Linux
sudo cp target/release/libsubstrait_textplan.so /usr/local/lib/
sudo ldconfig

# On macOS
sudo cp target/release/libsubstrait_textplan.dylib /usr/local/lib/
```

### Step 3: Get the Go Package

```bash
go get github.com/substrait-io/substrait-textplan/go/substrait
```

**Note**: The Go package expects the Rust library to be available at runtime. Make sure to follow Step 1 and Step 2 before using the package.

## Usage

```go
package main

import (
	"fmt"
	"log"

	"github.com/substrait-io/substrait-textplan/go/substrait"
)

func main() {
	// Create a new TextPlan instance
	tp := substrait.New()

	// Sample TextPlan
	textplan := `
	schema simple_schema {
		id i32;
		name string;
	}

	read RELATION data {
		SOURCE source;
		BASE_SCHEMA simple_schema;
	}

	ROOT {
		NAMES = [data]
	}
	`

	// Convert TextPlan to binary
	binary, err := tp.LoadFromText(textplan)
	if err != nil {
		log.Fatalf("Failed to convert TextPlan to binary: %v", err)
	}

	fmt.Printf("Converted TextPlan to binary (%d bytes)\n", len(binary))

	// Convert binary back to TextPlan
	roundTrip, err := tp.LoadFromBinary(binary)
	if err != nil {
		log.Fatalf("Failed to convert binary to TextPlan: %v", err)
	}

	fmt.Printf("Converted binary back to TextPlan:\n%s\n", roundTrip)
}
```

## Example Application

There's a simple example application in the `examples` directory that demonstrates how to convert between TextPlan and binary formats using files:

```bash
cd examples
go build simple.go

# Convert TextPlan to binary
./simple text2bin input.textplan output.bin

# Convert binary to TextPlan
./simple bin2text input.bin output.textplan
```

## License

Apache-2.0