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

	"context"
	"strings"

	"github.com/google/uuid"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"

	pb "orionis/proto"
)

// ── Config ────────────────────────────────────────────────────────────────────

// ── Config ────────────────────────────────────────────────────────────────────

// Config controls agent behaviour.
type Config struct {
	EngineURL       string
	GrpcURL         string
	UseGRPC         bool
	IncludeModules  []string
	ExcludeModules  []string
	Mode            string // "dev" | "safe" | "error"
	BatchSize       int
	FlushIntervalMs int
	MaxBufferSize   int
}

var defaultConfig = Config{
	EngineURL:       "http://localhost:7700",
	GrpcURL:         "localhost:7701",
	UseGRPC:         true,
	ExcludeModules:  []string{"runtime", "testing", "net/http"},
	Mode:            "dev",
	BatchSize:       20,
	FlushIntervalMs: 100,
	MaxBufferSize:   10000,
}

// ── Agent State ───────────────────────────────────────────────────────────────

type agent struct {
	cfg          Config
	mu           sync.Mutex
	batch        []TraceEvent
	stop         chan struct{}
	traceID      string
	sender       sender
	backoffUntil time.Time
	backoffCount int
}

type sendResult int

const (
	sendSuccess sendResult = iota
	sendRetryable
	sendFatal
)

type sender interface {
	send(events []TraceEvent) sendResult
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
	EventDbQuery       EventType = "db_query"
)

type LocalVar struct {
	Name     string `json:"name"`
	Value    string `json:"value"`
	TypeName string `json:"type_name"`
}

type TraceEvent struct {
	TraceID      string       `json:"trace_id"`
	SpanID       string       `json:"span_id"`
	ParentSpanID *string      `json:"parent_span_id"`
	TimestampMs  int64        `json:"timestamp_ms"`
	EventType    EventType    `json:"event_type"`
	FunctionName string       `json:"function_name"`
	Module       string       `json:"module"`
	File         string       `json:"file"`
	Line         int          `json:"line"`
	Locals       []LocalVar   `json:"locals,omitempty"`
	ErrorMessage *string      `json:"error_message,omitempty"`
	DurationUs   *int64       `json:"duration_us,omitempty"`
	Language     string       `json:"language"`
	ThreadID     string       `json:"thread_id"`
	HTTPRequest  *HTTPRequest `json:"http_request,omitempty"`
	DBQuery      *DBQuery     `json:"db_query,omitempty"`
}

type HTTPRequest struct {
	Method  string            `json:"method"`
	URL     string            `json:"url"`
	Headers map[string]string `json:"headers"`
	Body    []byte            `json:"body,omitempty"`
}

type DBQuery struct {
	Query      string `json:"query"`
	Driver     string `json:"driver"`
	DurationUs uint64 `json:"duration_us"`
}

// ── Public API ────────────────────────────────────────────────────────────────

// Start initialises the Orionis Go agent with the given config.
func Start(opts ...func(*Config)) {
	cfg := defaultConfig
	for _, o := range opts {
		o(&cfg)
	}

	var s sender
	if cfg.UseGRPC {
		gs, err := newGrpcSender(cfg.GrpcURL)
		if err != nil {
			fmt.Printf("[Orionis] gRPC connection failed: %v, falling back to HTTP\n", err)
			s = &httpSender{url: cfg.EngineURL}
		} else {
			s = gs
		}
	} else {
		s = &httpSender{url: cfg.EngineURL}
	}

	global = &agent{
		cfg:     cfg,
		stop:    make(chan struct{}),
		traceID: uuid.New().String(),
		sender:  s,
	}

	sendEnvSnapshot(cfg)
	go global.senderLoop()
	fmt.Printf("[Orionis] Go agent started — engine: %s | mode: %s | grpc: %v\n", cfg.EngineURL, cfg.Mode, cfg.UseGRPC)
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
func RecordPanic() {
	if r := recover(); r != nil {
		if global == nil {
			panic(r)
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
		panic(r)
	}
}

// CaptureQuery manually captures a database query event.
func CaptureQuery(query, driver string, durationUs uint64) {
	if global == nil {
		return
	}
	_, file, line, _ := runtime.Caller(1)
	global.enqueue(TraceEvent{
		TraceID:      global.traceID,
		SpanID:       uuid.New().String(),
		TimestampMs:  nowMs(),
		EventType:    EventDbQuery,
		FunctionName: "query",
		Module:       "db",
		File:         file,
		Line:         line,
		Language:     "go",
		DBQuery: &DBQuery{
			Query:      query,
			Driver:     driver,
			DurationUs: durationUs,
		},
	})
}

// InjectTraceHeaders adds W3C traceparent headers to an outgoing request.
func InjectTraceHeaders(req *http.Request) {
	if global == nil {
		return
	}
	global.mu.Lock()
	tid := global.traceID
	global.mu.Unlock()

	rawTid := strings.ReplaceAll(tid, "-", "")
	header := fmt.Sprintf("00-%s-0000000000000000-01", rawTid)
	req.Header.Set("traceparent", header)
}

// NewTraceID starts a new trace context.
func NewTraceID() string {
	if global == nil {
		return ""
	}
	global.mu.Lock()
	global.traceID = uuid.New().String()
	global.mu.Unlock()
	return global.traceID
}

// ── Internal ──────────────────────────────────────────────────────────────────

func (a *agent) enqueue(ev TraceEvent) {
	a.mu.Lock()
	defer a.mu.Unlock()

	if len(a.batch) >= a.cfg.MaxBufferSize {
		// Data shedding: drop oldest
		a.batch = a.batch[1:]
	}
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
			a.flush()
			if gs, ok := a.sender.(*grpcSender); ok {
				gs.conn.Close()
			}
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

	if time.Now().Before(a.backoffUntil) {
		a.mu.Unlock()
		return
	}

	toSend := make([]TraceEvent, len(a.batch))
	copy(toSend, a.batch)
	a.batch = a.batch[:0]
	a.mu.Unlock()

	res := a.sender.send(toSend)

	a.mu.Lock()
	defer a.mu.Unlock()

	switch res {
	case sendSuccess:
		a.backoffCount = 0
		a.backoffUntil = time.Time{}
	case sendRetryable:
		// Return to batch (prepend)
		if len(a.batch)+len(toSend) <= a.cfg.MaxBufferSize {
			a.batch = append(toSend, a.batch...)
		} else {
			// If combining overflows, just keep the new ones and some of the old ones
			space := a.cfg.MaxBufferSize - len(a.batch)
			if space > 0 {
				a.batch = append(toSend[len(toSend)-space:], a.batch...)
			}
		}

		// Exponential backoff
		a.backoffCount++
		delay := time.Duration(100*int(1<<uint(a.backoffCount))) * time.Millisecond
		if delay > 10*time.Second {
			delay = 10 * time.Second
		}
		a.backoffUntil = time.Now().Add(delay)
	case sendFatal:
		// Drop data
	}
}

// ── Senders ──────────────────────────────────────────────────────────────────

type httpSender struct {
	url string
}

func (s *httpSender) send(events []TraceEvent) sendResult {
	data, err := json.Marshal(events)
	if err != nil {
		return sendFatal
	}
	req, _ := http.NewRequest("POST", s.url+"/api/ingest", bytes.NewReader(data))
	req.Header.Set("Content-Type", "application/json")
	client := &http.Client{Timeout: 3 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return sendRetryable
	}
	defer resp.Body.Close()

	if resp.StatusCode == 503 || resp.StatusCode == 429 {
		return sendRetryable
	}
	if resp.StatusCode >= 400 && resp.StatusCode < 500 {
		return sendFatal
	}
	if resp.StatusCode >= 500 {
		return sendRetryable
	}
	return sendSuccess
}

type grpcSender struct {
	conn   *grpc.ClientConn
	client pb.IngestClient
}

func newGrpcSender(target string) (*grpcSender, error) {
	conn, err := grpc.Dial(target, grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		return nil, err
	}
	return &grpcSender{
		conn:   conn,
		client: pb.NewIngestClient(conn),
	}, nil
}

func (s *grpcSender) send(events []TraceEvent) sendResult {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	stream, err := s.client.StreamEvents(ctx)
	if err != nil {
		return sendRetryable
	}

	for _, ev := range events {
		pbEv := &pb.TraceEvent{
			TraceId:      ev.TraceID,
			SpanId:       ev.SpanID,
			TimestampMs:  uint64(ev.TimestampMs),
			FunctionName: ev.FunctionName,
			Module:       ev.Module,
			File:         ev.File,
			Line:         uint32(ev.Line),
			Language:     pb.AgentLanguage_LANG_GO,
		}

		if ev.ParentSpanID != nil {
			pbEv.ParentSpanId = *ev.ParentSpanID
		}
		if ev.ErrorMessage != nil {
			pbEv.ErrorMessage = *ev.ErrorMessage
		}
		if ev.DurationUs != nil {
			dur := uint64(*ev.DurationUs)
			pbEv.DurationUs = &dur
		}

		_ = stream.Send(pbEv)
	}
	_, err = stream.CloseAndRecv()
	if err != nil {
		// check grpc status
		if strings.Contains(err.Error(), "ResourceExhausted") || strings.Contains(err.Error(), "Unavailable") {
			return sendRetryable
		}
		return sendFatal
	}
	return sendSuccess
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
