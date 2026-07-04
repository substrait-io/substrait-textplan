//go:build ignore
// +build ignore

// SPDX-License-Identifier: Apache-2.0

// This file is run by go:generate to build the Rust library
package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
)

func main() {
	// Get the repository root (two directories up from this file)
	wd, err := os.Getwd()
	if err != nil {
		fmt.Fprintf(os.Stderr, "Failed to get working directory: %v\n", err)
		os.Exit(1)
	}

	repoRoot := filepath.Join(wd, "..", "..")

	fmt.Println("Building Substrait TextPlan Rust library...")
	fmt.Printf("Repository root: %s\n", repoRoot)

	// Check if cargo is available
	if _, err := exec.LookPath("cargo"); err != nil {
		fmt.Fprintln(os.Stderr, "ERROR: cargo not found in PATH")
		fmt.Fprintln(os.Stderr, "Please install Rust from https://rustup.rs/")
		os.Exit(1)
	}

	// Check if Java is available (for ANTLR)
	if _, err := exec.LookPath("java"); err != nil {
		fmt.Fprintln(os.Stderr, "ERROR: java not found in PATH")
		fmt.Fprintln(os.Stderr, "Java is required for ANTLR grammar generation")
		os.Exit(1)
	}

	// Build the Rust library
	cmd := exec.Command("cargo", "build", "--release")
	cmd.Dir = repoRoot
	cmd.Env = append(os.Environ(), "GENERATE_ANTLR=true")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr

	fmt.Println("Running: GENERATE_ANTLR=true cargo build --release")
	if err := cmd.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "Failed to build Rust library: %v\n", err)
		os.Exit(1)
	}

	fmt.Println("✓ Rust library built successfully")

	// Inform user about library path
	libPath := filepath.Join(repoRoot, "target", "release")
	var libName string
	if runtime.GOOS == "darwin" {
		libName = "libsubstrait_textplan.dylib"
	} else {
		libName = "libsubstrait_textplan.so"
	}

	fmt.Printf("\n✓ Library location: %s/%s\n", libPath, libName)
	fmt.Println("\nTo use the library, you may need to set:")
	if runtime.GOOS == "darwin" {
		fmt.Printf("  export DYLD_LIBRARY_PATH=%s:$DYLD_LIBRARY_PATH\n", libPath)
	} else {
		fmt.Printf("  export LD_LIBRARY_PATH=%s:$LD_LIBRARY_PATH\n", libPath)
	}
}
