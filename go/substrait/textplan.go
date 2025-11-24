// SPDX-License-Identifier: Apache-2.0

// Package substrait provides a Go wrapper for the Substrait TextPlan library
//
//go:generate go run build.go
package substrait

// #cgo LDFLAGS: -L${SRCDIR}/../../target/release -L${SRCDIR}/../../target/debug -L/usr/local/lib -lsubstrait_textplan
// #cgo darwin LDFLAGS: -Wl,-rpath,${SRCDIR}/../../target/release -Wl,-rpath,${SRCDIR}/../../target/debug -Wl,-rpath,/usr/local/lib
// #cgo linux LDFLAGS: -Wl,-rpath,${SRCDIR}/../../target/release -Wl,-rpath,${SRCDIR}/../../target/debug -Wl,-rpath,/usr/local/lib
// #include <stdlib.h>
// #include <stdint.h>
// #include <string.h>
// void* load_from_text(const char* text);
// void free_plan_bytes(void* ptr);
// char* save_to_text(const uint8_t* bytes, size_t bytes_len);
// void free_text_plan(char* text_ptr);
import "C"
import (
	"errors"
	"runtime"
	"unsafe"
)

// TextPlan represents a Substrait text plan
type TextPlan struct {
	// ensure the library stays loaded for the lifetime of the program
	// even if the GC runs
	keepAlive interface{}
}

// New creates a new TextPlan instance
func New() *TextPlan {
	return &TextPlan{}
}

// LoadFromText loads a textplan from a string and converts it to binary protobuf
func (tp *TextPlan) LoadFromText(text string) ([]byte, error) {
	cText := C.CString(text)
	defer C.free(unsafe.Pointer(cText))

	ptr := C.load_from_text(cText)
	if ptr == nil {
		return nil, errors.New("failed to load text plan")
	}

	// First sizeof(size_t) bytes contain the length
	lenPtr := (*C.size_t)(ptr)
	length := int(*lenPtr)

	// Rest is the data
	dataPtr := unsafe.Pointer(uintptr(unsafe.Pointer(ptr)) + unsafe.Sizeof(*lenPtr))

	// Copy the data to Go slice
	result := make([]byte, length)
	C.memcpy(unsafe.Pointer(&result[0]), dataPtr, C.size_t(length))

	// Free the C memory
	C.free_plan_bytes(ptr)

	return result, nil
}

// SaveToText loads a binary plan and converts it to textplan format
func (tp *TextPlan) SaveToText(data []byte) (string, error) {
	if len(data) == 0 {
		return "", errors.New("empty binary data")
	}

	cPtr := C.save_to_text((*C.uint8_t)(unsafe.Pointer(&data[0])), C.size_t(len(data)))
	if cPtr == nil {
		return "", errors.New("failed to convert binary plan to text")
	}

	// Convert C string to Go string
	result := C.GoString(cPtr)

	// Free the C memory
	C.free_text_plan(cPtr)

	return result, nil
}

// Helper functions for easier use

// LoadFromText loads a textplan from a string and converts it to binary protobuf
func LoadFromText(text string) ([]byte, error) {
	tp := New()
	return tp.LoadFromText(text)
}

// SaveToText loads a binary plan and converts it to textplan format
func SaveToText(data []byte) (string, error) {
	tp := New()
	return tp.SaveToText(data)
}

// Initialize the library automatically
func init() {
	// This ensures the library stays loaded
	runtime.KeepAlive(New())
}