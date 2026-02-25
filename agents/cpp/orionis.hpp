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
  // Socket handle type alias for cross-platform close
  #define ORIONIS_CLOSESOCKET(s) closesocket(s)
  using sock_t = SOCKET;
  static const sock_t INVALID_SOCK = INVALID_SOCKET;
#else
  #include <sys/socket.h>
  #include <netinet/in.h>
  #include <arpa/inet.h>
  #include <unistd.h>
  #define ORIONIS_CLOSESOCKET(s) ::close(s)
  using sock_t = int;
  static const sock_t INVALID_SOCK = -1;
#endif

namespace orionis {

// ── Config ────────────────────────────────────────────────────────────────────
inline std::string engine_url = "http://localhost:7700";
inline std::string trace_id;
inline std::atomic<bool> running{false};

// ── WSA init (Windows only, done once at start()) ─────────────────────────────
#ifdef _WIN32
inline WSADATA _wsa_data{};
inline bool    _wsa_initialized = false;
inline void _ensure_wsa() {
    if (!_wsa_initialized) {
        WSAStartup(MAKEWORD(2, 2), &_wsa_data);
        _wsa_initialized = true;
    }
}
#else
inline void _ensure_wsa() {}
#endif

// ── Utilities ─────────────────────────────────────────────────────────────────

inline uint64_t now_ms() {
    return static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::milliseconds>(
            std::chrono::system_clock::now().time_since_epoch()).count());
}

inline std::string new_uuid() {
    static std::atomic<uint64_t> seed{now_ms()};
    auto rand_hex = []() -> char {
        const char* digits = "0123456789abcdef";
        uint64_t v = seed.fetch_add(1);
        v ^= v << 13; v ^= v >> 7; v ^= v << 17; // xorshift
        seed = v;
        return digits[v % 16];
    };
    std::string u(36, '-');
    for (int i=0; i<36; i++) {
        if (i==8 || i==13 || i==18 || i==23) continue;
        u[i] = rand_hex();
    }
    u[14] = '4'; // version 4
    char c = u[19];
    if (c<'8' || c>'b') u[19] = '8'; // variant 1
    return u;
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

// ── HTTP POST (minimal, crash-safe, no external dependencies) ─────────────────
//
// Design choices for crash safety:
//  - WSAStartup is called ONCE in start() — never in signal handlers
//  - recv() loop waits for HTTP response so we know the server got the data
//  - SO_SNDTIMEO + SO_RCVTIMEO: 2-second timeouts; we can't hang in a crash handler
//  - Returns true only if HTTP 200 received

inline bool post_json(const std::string& host, int port,
                      const std::string& path, const std::string& body) {
    // _ensure_wsa() was already called in start() — safe to skip here
    // but we call it defensively in case someone calls flush before start().
    _ensure_wsa();

    sock_t sock = socket(AF_INET, SOCK_STREAM, 0);
    if (sock == INVALID_SOCK) return false;

    // Set send + receive timeouts (2 seconds each)
#ifdef _WIN32
    DWORD tv = 2000;
    setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, (const char*)&tv, sizeof(tv));
    setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, (const char*)&tv, sizeof(tv));
#else
    struct timeval tv{2, 0};
    setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv));
    setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
#endif

    struct sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_port   = htons(port);
    addr.sin_addr.s_addr = inet_addr(host.c_str());

    if (connect(sock, (struct sockaddr*)&addr, sizeof(addr)) < 0) {
        fprintf(stderr, "[Orionis] Connect failed to %s:%d\n", host.c_str(), port);
        ORIONIS_CLOSESOCKET(sock);
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

    // Send full request
    size_t total_sent = 0;
    while (total_sent < r.size()) {
        int sent = send(sock, r.c_str() + total_sent, (int)(r.size() - total_sent), 0);
        if (sent <= 0) {
            fprintf(stderr, "[Orionis] Send error at byte %zu\n", total_sent);
            ORIONIS_CLOSESOCKET(sock);
            return false;
        }
        total_sent += sent;
    }

    // Read response to confirm server received data (critical in crash path)
    // We only need the first line to check for "200 OK"
    char resp_buf[256] = {};
    bool got_200 = false;
    int bytes = recv(sock, resp_buf, sizeof(resp_buf) - 1, 0);
    if (bytes > 0) {
        resp_buf[bytes] = '\0';
        got_200 = (strstr(resp_buf, "200") != nullptr);
        fprintf(stderr, "[Orionis] Engine response: %s\n",
                got_200 ? "200 OK" : resp_buf);
    }

    ORIONIS_CLOSESOCKET(sock);
    return got_200;
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
//
// NOTE: post_json is technically async-signal-unsafe (uses printf, std::string).
// For a production-grade agent this would be a pre-allocated fixed buffer write.
// For v0.1 this is the correct pragmatic approach — gets the crash into the engine.

inline void crash_handler(int sig) {
    std::string name;
    switch (sig) {
        case SIGSEGV: name = "SIGSEGV (Segmentation Fault)"; break;
        case SIGABRT: name = "SIGABRT (Abort)"; break;
        case SIGFPE:  name = "SIGFPE (Floating Point Exception)"; break;
        case SIGILL:  name = "SIGILL (Illegal Instruction)"; break;
        default:      name = "Signal " + std::to_string(sig); break;
    }
    Event ev;
    ev.trace_id     = trace_id;
    ev.span_id      = new_uuid();
    ev.timestamp_ms = now_ms();
    ev.event_type   = "exception";
    ev.function_name= "crash";
    ev.file         = "runtime";
    ev.line         = 0;
    ev.error_message= name;
    ev.duration_us  = -1;
    enqueue(ev);
    fprintf(stderr, "[Orionis] Caught crash signal %d (%s). Flushing...\n",
            sig, name.c_str());
    flush();

    // Restore default and re-raise so the OS records the correct exit code
    std::signal(sig, SIG_DFL);
    std::raise(sig);
}

inline void terminate_handler() {
    Event ev;
    ev.trace_id     = trace_id;
    ev.span_id      = new_uuid();
    ev.timestamp_ms = now_ms();
    ev.event_type   = "exception";
    ev.function_name= "terminate";
    ev.file         = "runtime";
    ev.line         = 0;
    ev.error_message= "C++ std::terminate() called (unhandled exception)";
    ev.duration_us  = -1;
    enqueue(ev);
    fprintf(stderr, "[Orionis] Caught std::terminate(). Flushing...\n");
    flush();
    std::abort();
}

#ifdef _WIN32
inline LONG WINAPI win32_exception_handler(EXCEPTION_POINTERS* ExceptionInfo) {
    // Skip C++ exceptions — those are routed through std::terminate above
    if (ExceptionInfo->ExceptionRecord->ExceptionCode == 0xE06D7363) {
        return EXCEPTION_CONTINUE_SEARCH;  // 0xE06D7363 = C++ exception magic
    }

    std::string name = "Windows Exception: 0x";
    char hex[20];
    sprintf(hex, "%08X", (unsigned int)ExceptionInfo->ExceptionRecord->ExceptionCode);
    name += hex;

    Event ev;
    ev.trace_id     = trace_id;
    ev.span_id      = new_uuid();
    ev.timestamp_ms = now_ms();
    ev.event_type   = "exception";
    ev.function_name= "crash";
    ev.file         = "runtime";
    ev.line         = 0;
    ev.error_message= name;
    ev.duration_us  = -1;
    enqueue(ev);
    fprintf(stderr, "[Orionis] Win32 exception: %s. Flushing...\n", name.c_str());
    flush();

    return EXCEPTION_CONTINUE_SEARCH;
}
#endif

// ── Public API ────────────────────────────────────────────────────────────────

inline void start(const std::string& url = "http://localhost:7700") {
    engine_url = url;
    trace_id   = new_uuid();
    running    = true;

    // Init Winsock ONCE — safe for subsequent calls from signal handlers
    _ensure_wsa();

    // Register POSIX crash signal handlers
    std::signal(SIGSEGV, crash_handler);
    std::signal(SIGABRT, crash_handler);
    std::signal(SIGFPE,  crash_handler);
    std::signal(SIGILL,  crash_handler);

    // Register C++ unhandled exception handler
    std::set_terminate(terminate_handler);

#ifdef _WIN32
    // Register Win32 structured exception handler (catches access violations etc.)
    AddVectoredExceptionHandler(1, win32_exception_handler);
#endif

    // Background flush thread (100ms interval for normal operation)
    std::thread([]() {
        while (running) {
            std::this_thread::sleep_for(std::chrono::milliseconds(100));
            flush();
        }
        flush();  // final flush on stop()
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
    std::chrono::steady_clock::time_point start_tp;

    SpanGuard(const char* fn, const char* f, int l)
        : span_id(new_uuid()), fn_name(fn), file(f), line(l),
          start_tp(std::chrono::steady_clock::now()) {
        Event ev;
        ev.trace_id      = trace_id;
        ev.span_id       = span_id;
        ev.timestamp_ms  = now_ms();
        ev.event_type    = "function_enter";
        ev.function_name = fn_name;
        ev.file          = file;
        ev.line          = line;
        ev.duration_us   = -1;
        enqueue(ev);
    }

    ~SpanGuard() {
        auto dur = std::chrono::duration_cast<std::chrono::microseconds>(
            std::chrono::steady_clock::now() - start_tp).count();
        Event ev;
        ev.trace_id       = trace_id;
        ev.span_id        = new_uuid();
        ev.parent_span_id = span_id;
        ev.timestamp_ms   = now_ms();
        ev.event_type     = "function_exit";
        ev.function_name  = fn_name;
        ev.file           = file;
        ev.line           = line;
        ev.duration_us    = dur;
        enqueue(ev);
    }
};

} // namespace orionis

// ── Macro ─────────────────────────────────────────────────────────────────────

/// Place ORION_TRACE() at the top of any C++ function to instrument it.
#define ORION_TRACE() \
    orionis::SpanGuard _orion_guard_(__FUNCTION__, __FILE__, __LINE__)
