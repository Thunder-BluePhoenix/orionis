package main

import (
	"fmt"
	"net/http"
	"time"

	"orionis"
)

func buggyHandler(w http.ResponseWriter, r *http.Request) {
	defer orionis.RecordPanic()
	fmt.Println("Handling request...")
	var slice []int
	// Deliberate panic: slice index out of bounds
	_ = slice[0]
}

func main() {
	orionis.Start(
		orionis.WithEngine("http://localhost:7700"),
		orionis.WithMode("dev"),
	)
	defer orionis.Stop()

	mux := http.NewServeMux()
	mux.HandleFunc("/", buggyHandler)

	handler := orionis.HTTPMiddleware(mux)

	fmt.Println("Server listening on :8080. Test panic in a few seconds...")

	// Simulate a request after 1 second
	go func() {
		// Wait a small moment to ensure tracing context starts correctly
		time.Sleep(500 * time.Millisecond)
		// We'll just run it synchronously by calling it
		req, _ := http.NewRequest("GET", "/", nil)
		handler.ServeHTTP(nil, req)
	}()

	// Wait a bit to let the trace flush
	// (Simulating a long-running server but we just want the panic to record)
	time.Sleep(2 * time.Second)
}
