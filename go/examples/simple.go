// SPDX-License-Identifier: Apache-2.0

// Example program for using the Substrait TextPlan Go wrapper
package main

import (
	"fmt"
	"io/ioutil"
	"log"
	"os"

	"github.com/substrait-io/substrait-textplan/go/substrait"
)

func main() {
	if len(os.Args) < 2 {
		fmt.Println("Usage: simple <command> [args...]")
		fmt.Println("Commands:")
		fmt.Println("  text2bin <input.textplan> <output.bin>  - Convert TextPlan to binary protobuf")
		fmt.Println("  bin2text <input.bin> <output.textplan>  - Convert binary protobuf to TextPlan")
		os.Exit(1)
	}

	command := os.Args[1]

	switch command {
	case "text2bin":
		if len(os.Args) < 4 {
			log.Fatal("Missing input or output file arguments")
		}
		convertTextToBinary(os.Args[2], os.Args[3])
	case "bin2text":
		if len(os.Args) < 4 {
			log.Fatal("Missing input or output file arguments")
		}
		convertBinaryToText(os.Args[2], os.Args[3])
	default:
		log.Fatalf("Unknown command: %s", command)
	}
}

func convertTextToBinary(inputFile, outputFile string) {
	// Read the input TextPlan file
	textData, err := ioutil.ReadFile(inputFile)
	if err != nil {
		log.Fatalf("Failed to read input file: %v", err)
	}

	// Create a TextPlan instance
	tp := substrait.New()

	// Convert TextPlan to binary
	binaryData, err := tp.LoadFromText(string(textData))
	if err != nil {
		log.Fatalf("Failed to convert TextPlan to binary: %v", err)
	}

	// Write the binary data to the output file
	err = ioutil.WriteFile(outputFile, binaryData, 0644)
	if err != nil {
		log.Fatalf("Failed to write output file: %v", err)
	}

	fmt.Printf("Successfully converted %s to binary format at %s\n", inputFile, outputFile)
}

func convertBinaryToText(inputFile, outputFile string) {
	// Read the input binary file
	binaryData, err := ioutil.ReadFile(inputFile)
	if err != nil {
		log.Fatalf("Failed to read input file: %v", err)
	}

	// Create a TextPlan instance
	tp := substrait.New()

	// Convert binary to TextPlan
	textData, err := tp.SaveToText(binaryData)
	if err != nil {
		log.Fatalf("Failed to convert binary to TextPlan: %v", err)
	}

	// Write the TextPlan to the output file
	err = ioutil.WriteFile(outputFile, []byte(textData), 0644)
	if err != nil {
		log.Fatalf("Failed to write output file: %v", err)
	}

	fmt.Printf("Successfully converted %s to TextPlan format at %s\n", inputFile, outputFile)
}