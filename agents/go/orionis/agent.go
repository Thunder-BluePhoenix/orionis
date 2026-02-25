// Package orionis provides a zero-config runtime tracing agent for Go applications.
// It hooks into goroutine execution, captures function calls, panics, and HTTP requests,
// and streams events to the Orionis engine.
package orionis

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"sync"
	"time"

	"github.com/google/uuid"
)

// ── Config ────────────────────────────────────────────────────────────────────

// Config controls agent behaviour.
type Config struct {
	EngineURL       string
	IncludeModules  []string
	ExcludeModules  []string
	Mode            string // "dev" | "safe" | "error"
	BatchSize       int
	FlushIntervalMs int
}

var defaultConfig = Config{
	EngineURL:       "http://localhost:7700",
	ExcludeModules:  []string{"runtime", "testing", "net/http"},
	Mode:            "dev",
	BatchSize:       20,
	FlushIntervalMs: 100,
}

// ── Agent State ───────────────────────────────────────────────────────────────

type agent struct {
	cfg     Config
	mu      sync.Mutex
	batch   []TraceEvent
	stop    chan struct{}
	traceID string
}

var global *agent

// ── Models ────────────────────────────────────────────────────────────────────

type EventType string

const (
	EventFunctionEnter EventType = "function_enter"
	EventFunctionExit  EventType = "function_exit"
	EventException     EventType = "exception"
	EventHttpRequest   EventType = "http_request"
	EventHttpResponse  EventType = "http_response"
)

type LocalVar struct {
	Name     string `json:"name"`
	Value    string `json:"value"`
	TypeName string `json:"type_name"`
}

type TraceEvent struct {
	TraceID      string     `json:"trace_id"`
	SpanID       string     `json:"span_id"`
	ParentSpanID *string    `json:"parent_span_id"`
	TimestampMs  int64      `json:"timestamp_ms"`
	EventType    EventType  `json:"event_type"`
	FunctionName string     `json:"function_name"`
	Module       string     `json:"module"`
	File         string     `json:"file"`
	Line         int        `json:"line"`
	Locals       []LocalVar `json:"locals,omitempty"`
	ErrorMessage *string    `json:"error_message,omitempty"`
	DurationUs   *int64     `json:"duration_us,omitempty"`
	Language     string     `json:"language"`
}

// ── Public API ────────────────────────────────────────────────────────────────

// Start initialises the Orionis Go agent with the given config.
func Start(opts ...func(*Config)) {
	cfg := defaultConfig
	for _, o := range opts {
		o(&cfg)
	}
	global = &agent{cfg: cfg, stop: make(chan struct{}), traceID: uuid.New().String()}
	sendEnvSnapshot(cfg)
	go global.senderLoop()
	fmt.Printf("[Orionis] Go agent started — engine: %s | mode: %s\n", cfg.EngineURL, cfg.Mode)
}

// Stop flushes remaining events and shuts down the agent.
func Stop() {
	if global == nil {
		return
	}
	close(global.stop)
	global.flush()
	fmt.Println("[Orionis] Go agent stopped.")
}

// WithEngine sets the engine URL.
func WithEngine(url string) func(*Config) { return func(c *Config) { c.EngineURL = url } }

// WithModules sets the include module filter.
func WithModules(mods ...string) func(*Config) {
	return func(c *Config) { c.IncludeModules = mods }
}

// WithMode sets the tracing mode (dev|safe|error).
func WithMode(mode string) func(*Config) { return func(c *Config) { c.Mode = mode } }

// Trace captures a function call manually (use defer for exit).
// Returns a function to call on function exit (capture return timing).
//
// Usage:
//
//	func MyFunc() {
//	    defer orionis.Trace()()
//	    ...
//	}
func Trace() func() {
	if global == nil {
		return func() {}
	}
	pc, file, line, ok := runtime.Caller(1)
	if !ok {
		return func() {}
	}
	fn := runtime.FuncForPC(pc)
	fnName := "unknown"
	module := ""
	if fn != nil {
		fnName = filepath.Base(fn.Name())
		module = fn.Name()
	}

	spanID := uuid.New().String()
	startNs := time.Now().UnixNano()

	global.enqueue(TraceEvent{
		TraceID:      global.traceID,
		SpanID:       spanID,
		TimestampMs:  nowMs(),
		EventType:    EventFunctionEnter,
		FunctionName: fnName,
		Module:       module,
		File:         file,
		Line:         line,
		Language:     "go",
	})

	return func() {
		durUs := (time.Now().UnixNano() - startNs) / 1000
		exitSpan := uuid.New().String()
		global.enqueue(TraceEvent{
			TraceID:      global.traceID,
			SpanID:       exitSpan,
			ParentSpanID: &spanID,
			TimestampMs:  nowMs(),
			EventType:    EventFunctionExit,
			FunctionName: fnName,
			Module:       module,
			File:         file,
			Line:         line,
			DurationUs:   &durUs,
			Language:     "go",
		})
	}
}

// RecordPanic should be used with defer to capture panic information.
//
// Usage:
//
//	defer orionis.RecordPanic()
func RecordPanic() {
	if r := recover(); r != nil {
		if global == nil {
			panic(r) // re-panic if agent not running
		}
		pc, file, line, _ := runtime.Caller(2)
		fn := runtime.FuncForPC(pc)
		fnName := "unknown"
		if fn != nil {
			fnName = filepath.Base(fn.Name())
		}
		errMsg := fmt.Sprintf("panic: %v", r)
		global.enqueue(TraceEvent{
			TraceID:      global.traceID,
			SpanID:       uuid.New().String(),
			TimestampMs:  nowMs(),
			EventType:    EventException,
			FunctionName: fnName,
			File:         file,
			Line:         line,
			ErrorMessage: &errMsg,
			Language:     "go",
		})
		global.flush()
		panic(r) // re-panic so the program still crashes normally
	}
}

// HTTPMiddleware wraps an http.Handler to trace incoming requests.
func HTTPMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if global == nil {
			next.ServeHTTP(w, r)
			return
		}
		spanID := uuid.New().String()
		path := r.URL.Path
		method := r.Method
		start := time.Now()

		global.enqueue(TraceEvent{
			TraceID:      global.traceID,
			SpanID:       spanID,
			TimestampMs:  nowMs(),
			EventType:    EventHttpRequest,
			FunctionName: method + " " + path,
			Module:       "net/http",
			File:         "middleware",
			Line:         0,
			Language:     "go",
		})

		next.ServeHTTP(w, r)

		durUs := time.Since(start).Microseconds()
		global.enqueue(TraceEvent{
			TraceID:      global.traceID,
			SpanID:       uuid.New().String(),
			ParentSpanID: &spanID,
			TimestampMs:  nowMs(),
			EventType:    EventHttpResponse,
			FunctionName: method + " " + path,
			Module:       "net/http",
			File:         "middleware",
			Line:         0,
			DurationUs:   &durUs,
			Language:     "go",
		})
	})
}

// NewTraceID starts a new trace context (call at the start of each request/job).
func NewTraceID() string {
	if global == nil {
		return ""
	}
	global.traceID = uuid.New().String()
	return global.traceID
}

// ── Internal ──────────────────────────────────────────────────────────────────

func (a *agent) enqueue(ev TraceEvent) {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.batch = append(a.batch, ev)
}

func (a *agent) senderLoop() {
	ticker := time.NewTicker(time.Duration(a.cfg.FlushIntervalMs) * time.Millisecond)
	defer ticker.Stop()
	for {
		select {
		case <-ticker.C:
			a.flush()
		case <-a.stop:
			return
		}
	}
}

func (a *agent) flush() {
	a.mu.Lock()
	if len(a.batch) == 0 {
		a.mu.Unlock()
		return
	}
	toSend := make([]TraceEvent, len(a.batch))
	copy(toSend, a.batch)
	a.batch = a.batch[:0]
	a.mu.Unlock()

	data, err := json.Marshal(toSend)
	if err != nil {
		return
	}
	req, _ := http.NewRequest("POST", a.cfg.EngineURL+"/api/ingest", bytes.NewReader(data))
	req.Header.Set("Content-Type", "application/json")
	client := &http.Client{Timeout: 3 * time.Second}
	client.Do(req) // nolint: errcheck — drop on failure
}

func sendEnvSnapshot(cfg Config) {
	hostname, _ := os.Hostname()
	wd, _ := os.Getwd()
	snap := map[string]interface{}{
		"os": runtime.GOOS, "arch": runtime.GOARCH,
		"hostname": hostname, "go_version": runtime.Version(),
		"working_dir": wd, "captured_at": nowMs(),
	}
	data, _ := json.Marshal(map[string]interface{}{"env_snapshot": snap})
	req, _ := http.NewRequest("POST", cfg.EngineURL+"/api/ingest", bytes.NewReader(data))
	req.Header.Set("Content-Type", "application/json")
	client := &http.Client{Timeout: 2 * time.Second}
	client.Do(req) // nolint: errcheck
}

func nowMs() int64 { return time.Now().UnixMilli() }
