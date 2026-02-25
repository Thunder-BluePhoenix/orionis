/// orionis.hpp — Orionis C++ tracing agent (header-only)
/// 
/// Usage:
///   #include "orionis.hpp"
///
///   int main() {
///       orionis::start("http://localhost:7700");
///       ORION_TRACE();   // in any function
///   }

#pragma once

#include <string>
#include <vector>
#include <map>
#include <thread>
#include <mutex>
#include <atomic>
#include <deque>
#include <sstream>
#include <chrono>
#include <csignal>
#include <cstdlib>
#include <functional>
#include <cstring>

#ifdef _WIN32
  #include <winsock2.h>
  #include <windows.h>
  #pragma comment(lib, "ws2_32.lib")
#else
  #include <sys/socket.h>
  #include <netinet/in.h>
  #include <arpa/inet.h>
  #include <unistd.h>
#endif

namespace orionis {

// ── Config ────────────────────────────────────────────────────────────────────
inline std::string engine_url = "http://localhost:7700";
inline std::string trace_id;
inline std::atomic<bool> running{false};

// ── Utilities ─────────────────────────────────────────────────────────────────

inline uint64_t now_ms() {
    return static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::milliseconds>(
            std::chrono::system_clock::now().time_since_epoch()).count());
}

inline std::string new_uuid() {
    // Simple pseudo-UUID using time + counter
    static std::atomic<uint64_t> counter{0};
    auto t = now_ms();
    auto c = counter.fetch_add(1);
    std::ostringstream ss;
    ss << std::hex << t << "-" << c << "-orionis";
    return ss.str();
}

// ── Event ─────────────────────────────────────────────────────────────────────

struct Event {
    std::string trace_id;
    std::string span_id;
    std::string parent_span_id;
    uint64_t    timestamp_ms;
    std::string event_type;
    std::string function_name;
    std::string module;
    std::string file;
    int         line;
    std::string error_message;
    int64_t     duration_us;   // -1 = not set
};

inline std::string escape_json(const std::string& s) {
    std::string out;
    for (char c : s) {
        if (c == '"')  out += "\\\"";
        else if (c == '\\') out += "\\\\";
        else if (c == '\n') out += "\\n";
        else if (c == '\r') out += "\\r";
        else out += c;
    }
    return out;
}

inline std::string event_to_json(const Event& e) {
    std::ostringstream j;
    j << "{"
      << "\"trace_id\":\"" << e.trace_id << "\","
      << "\"span_id\":\"" << e.span_id << "\","
      << "\"parent_span_id\":" << (e.parent_span_id.empty() ? "null" : "\"" + e.parent_span_id + "\"") << ","
      << "\"timestamp_ms\":" << e.timestamp_ms << ","
      << "\"event_type\":\"" << e.event_type << "\","
      << "\"function_name\":\"" << escape_json(e.function_name) << "\","
      << "\"module\":\"" << escape_json(e.module) << "\","
      << "\"file\":\"" << escape_json(e.file) << "\","
      << "\"line\":" << e.line << ","
      << "\"error_message\":" << (e.error_message.empty() ? "null" : "\"" + escape_json(e.error_message) + "\"") << ","
      << "\"duration_us\":" << (e.duration_us < 0 ? "null" : std::to_string(e.duration_us)) << ","
      << "\"language\":\"cpp\""
      << "}";
    return j.str();
}

// ── Batch Queue ───────────────────────────────────────────────────────────────

inline std::mutex   batch_mu;
inline std::deque<Event> batch;

inline void enqueue(Event ev) {
    std::lock_guard<std::mutex> lk(batch_mu);
    batch.push_back(std::move(ev));
}

// ── HTTP POST (minimal, no dependencies) ──────────────────────────────────────

inline bool post_json(const std::string& host, int port, const std::string& path, const std::string& body) {
#ifdef _WIN32
    WSADATA wsa; WSAStartup(MAKEWORD(2,2), &wsa);
#endif
    int sock = socket(AF_INET, SOCK_STREAM, 0);
    if (sock < 0) return false;
    struct sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_port   = htons(port);
    addr.sin_addr.s_addr = inet_addr(host.c_str());
    if (connect(sock, (struct sockaddr*)&addr, sizeof(addr)) < 0) {
        fprintf(stderr, "[Orionis DEBUG] Connect failed to %s:%d\n", host.c_str(), port);
#ifdef _WIN32
        closesocket(sock);
#else
        close(sock);
#endif
        return false;
    }
    std::ostringstream req;
    req << "POST " << path << " HTTP/1.1\r\n"
        << "Host: " << host << ":" << port << "\r\n"
        << "Content-Type: application/json\r\n"
        << "Content-Length: " << body.size() << "\r\n"
        << "Connection: close\r\n"
        << "\r\n"
        << body;
    auto r = req.str();
    int sent_bytes = send(sock, r.c_str(), (int)r.size(), 0);
    if (sent_bytes < 0) {
        fprintf(stderr, "[Orionis DEBUG] Send failed\n");
    } else {
        fprintf(stderr, "[Orionis DEBUG] Sent %d bytes to %s\n", sent_bytes, path.c_str());
    }
#ifdef _WIN32
    closesocket(sock);
#else
    close(sock);
#endif
    return true;
}

inline void flush() {
    std::vector<Event> to_send;
    {
        std::lock_guard<std::mutex> lk(batch_mu);
        if (batch.empty()) return;
        to_send.assign(batch.begin(), batch.end());
        batch.clear();
    }
    std::string body = "[";
    for (size_t i = 0; i < to_send.size(); ++i) {
        body += event_to_json(to_send[i]);
        if (i + 1 < to_send.size()) body += ",";
    }
    body += "]";
    post_json("127.0.0.1", 7700, "/api/ingest", body);
}

// ── Signal / Crash Handler ────────────────────────────────────────────────────

inline std::function<void()> prev_abort;

inline void crash_handler(int sig) {
    std::string name;
    switch (sig) {
        case SIGSEGV: name = "SIGSEGV (Segmentation Fault)"; break;
        case SIGABRT: name = "SIGABRT (Abort)"; break;
        case SIGFPE:  name = "SIGFPE (Floating Point Exception)"; break;
        default:      name = "Signal " + std::to_string(sig); break;
    }
    Event ev;
    ev.trace_id    = trace_id;
    ev.span_id     = new_uuid();
    ev.timestamp_ms= now_ms();
    ev.event_type  = "exception";
    ev.function_name = "crash";
    ev.file        = "runtime";
    ev.line        = 0;
    ev.error_message = name;
    ev.duration_us = -1;
    enqueue(ev);
    fprintf(stderr, "[Orionis DEBUG] Caught crash signal %d. Flushing...\n", sig);
    flush();
    std::signal(sig, SIG_DFL);
    std::raise(sig);
}

inline void terminate_handler() {
    Event ev;
    ev.trace_id    = trace_id;
    ev.span_id     = new_uuid();
    ev.timestamp_ms= now_ms();
    ev.event_type  = "exception";
    ev.function_name = "terminate";
    ev.file        = "runtime";
    ev.line        = 0;
    ev.error_message = "C++ std::terminate() called (unhandled exception)";
    ev.duration_us = -1;
    enqueue(ev);
    fprintf(stderr, "[Orionis DEBUG] Caught std::terminate(). Flushing...\n");
    flush();
    
    // Call the original abort to exit
    std::abort();
}

#ifdef _WIN32
inline LONG WINAPI win32_exception_handler(EXCEPTION_POINTERS* ExceptionInfo) {
#ifdef _WIN32
    WSADATA wsa; WSAStartup(MAKEWORD(2,2), &wsa);
#endif

    std::string name = "Windows Exception: 0x";
    char hex[20];
    sprintf(hex, "%08X", (unsigned int)ExceptionInfo->ExceptionRecord->ExceptionCode);
    name += hex;
    
    Event ev;
    ev.trace_id    = trace_id;
    ev.span_id     = new_uuid();
    ev.timestamp_ms= now_ms();
    ev.event_type  = "exception";
    ev.function_name = "crash";
    ev.file        = "runtime";
    ev.line        = 0;
    ev.error_message = name;
    ev.duration_us = -1;
    enqueue(ev);
    flush();
    
    return EXCEPTION_CONTINUE_SEARCH;
}
#endif

// ── Public API ────────────────────────────────────────────────────────────────

inline void start(const std::string& url = "http://localhost:7700") {
    engine_url = url;
    trace_id   = new_uuid();
    running    = true;

    // Register crash handlers
    std::signal(SIGSEGV, crash_handler);
    std::signal(SIGABRT, crash_handler);
    std::signal(SIGFPE,  crash_handler);
    
    // Register standard C++ termination handler
    std::set_terminate(terminate_handler);

#ifdef _WIN32
    AddVectoredExceptionHandler(1, win32_exception_handler);
#endif

    // Background flush thread
    std::thread([]() {
        while (running) {
            std::this_thread::sleep_for(std::chrono::milliseconds(100));
            flush();
        }
        flush();
    }).detach();

    fprintf(stderr, "[Orionis] C++ agent started — engine: %s\n", url.c_str());
}

inline void stop() {
    running = false;
    flush();
    fprintf(stderr, "[Orionis] C++ agent stopped.\n");
}

// ── RAII Span Guard ───────────────────────────────────────────────────────────

class SpanGuard {
public:
    std::string span_id;
    std::string fn_name;
    std::string file;
    int         line;
    std::chrono::steady_clock::time_point start;

    SpanGuard(const char* fn, const char* f, int l)
        : span_id(new_uuid()), fn_name(fn), file(f), line(l),
          start(std::chrono::steady_clock::now()) {
        Event ev;
        ev.trace_id     = trace_id;
        ev.span_id      = span_id;
        ev.timestamp_ms = now_ms();
        ev.event_type   = "function_enter";
        ev.function_name= fn_name;
        ev.file         = file;
        ev.line         = line;
        ev.duration_us  = -1;
        enqueue(ev);
    }

    ~SpanGuard() {
        auto dur = std::chrono::duration_cast<std::chrono::microseconds>(
            std::chrono::steady_clock::now() - start).count();
        Event ev;
        ev.trace_id      = trace_id;
        ev.span_id       = new_uuid();
        ev.parent_span_id= span_id;
        ev.timestamp_ms  = now_ms();
        ev.event_type    = "function_exit";
        ev.function_name = fn_name;
        ev.file          = file;
        ev.line          = line;
        ev.duration_us   = dur;
        enqueue(ev);
    }
};

} // namespace orionis

// ── Macro ─────────────────────────────────────────────────────────────────────

/// Place ORION_TRACE() at the top of any C++ function to instrument it.
#define ORION_TRACE() \
    orionis::SpanGuard _orion_guard_(__FUNCTION__, __FILE__, __LINE__)
