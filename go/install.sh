#!/bin/bash
# SPDX-License-Identifier: Apache-2.0

# Installation script for Substrait TextPlan Go bindings
# This script builds the Rust library and sets up the Go package

set -e

echo "=== Substrait TextPlan Go Bindings Installation ==="
echo

# Check prerequisites
echo "Checking prerequisites..."

if ! command -v cargo &> /dev/null; then
    echo "ERROR: cargo not found. Please install Rust from https://rustup.rs/"
    exit 1
fi

if ! command -v java &> /dev/null; then
    echo "ERROR: java not found. Please install Java (required for ANTLR)"
    exit 1
fi

if ! command -v go &> /dev/null; then
    echo "ERROR: go not found. Please install Go from https://go.dev/dl/"
    exit 1
fi

echo "✓ All prerequisites found"
echo

# Build the Rust library
echo "Building Rust library..."
cd ..
GENERATE_ANTLR=true cargo build --release
echo "✓ Rust library built"
echo

# Determine library path
LIB_PATH="$(pwd)/target/release"
if [[ "$OSTYPE" == "darwin"* ]]; then
    LIB_FILE="libsubstrait_textplan.dylib"
    ENV_VAR="DYLD_LIBRARY_PATH"
else
    LIB_FILE="libsubstrait_textplan.so"
    ENV_VAR="LD_LIBRARY_PATH"
fi

echo "=== Installation Complete ==="
echo
echo "Library location: $LIB_PATH/$LIB_FILE"
echo
echo "To use the Go package, add this to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
echo "  export $ENV_VAR=$LIB_PATH:\$$ENV_VAR"
echo
echo "Or run it now in your current shell:"
echo "  export $ENV_VAR=$LIB_PATH:\$$ENV_VAR"
echo
echo "Then you can use the package:"
echo "  go get github.com/substrait-io/substrait-textplan/go/substrait"
echo
