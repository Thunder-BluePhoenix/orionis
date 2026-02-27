/**
 * orionis.h — Orionis C tracing agent (header-only, pure C11)
 * 
 * Usage:
 *   #include "orionis.h"
 * 
 *   int main() {
 *       orionis_start("http://localhost:7700");
 *       ORIONIS_TRACE("main");
 *       // ... code ...
 *       orionis_stop();
 *   }
 */

#ifndef ORIONIS_H
#define ORIONIS_H

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <time.h>
#include <stdarg.h>

#ifdef _WIN32
  #include <winsock2.h>
  #include <windows.h>
  #pragma comment(lib, "ws2_32.lib")
  #define ORIONIS_CLOSESOCKET(s) closesocket(s)
  typedef SOCKET orionis_sock_t;
#else
  #include <sys/socket.h>
  #include <netinet/in.h>
  #include <arpa/inet.h>
  #include <unistd.h>
  #include <pthread.h>
  #define ORIONIS_CLOSESOCKET(s) close(s)
  typedef int orionis_sock_t;
#endif

// ── Configuration ──────────────────────────────────────────────────────────

static char orionis_engine_url[256] = "http://localhost:7700";
static char orionis_engine_host[128] = "127.0.0.1";
static int  orionis_engine_port = 7700;

#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
  #define ORIONIS_THREAD_LOCAL _Thread_local
#elif defined(_MSC_VER)
  #define ORIONIS_THREAD_LOCAL __declspec(thread)
#else
  #define ORIONIS_THREAD_LOCAL __thread
#endif

static ORIONIS_THREAD_LOCAL char orionis_tid[64] = {0};

// ── GUID / Time Utilities ──────────────────────────────────────────────────

static uint64_t orionis_now_ms() {
#ifdef _WIN32
    FILETIME ft;
    GetSystemTimeAsFileTime(&ft);
    uint64_t nanoseconds = ((uint64_t)ft.dwHighDateTime << 32 | ft.dwLowDateTime) * 100;
    return nanoseconds / 1000000;
#else
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (uint64_t)ts.tv_sec * 1000 + (uint64_t)ts.tv_nsec / 1000000;
#endif
}

static void orionis_new_uuid(char* out) {
    const char* chars = "0123456789abcdef";
    for (int i = 0; i < 36; i++) {
        if (i == 8 || i == 13 || i == 18 || i == 23) {
            out[i] = '-';
        } else {
            out[i] = chars[rand() % 16];
        }
    }
    out[14] = '4'; // version 4
    out[36] = '\0';
}

// ── Ingestion ──────────────────────────────────────────────────────────────

static void orionis_post(const char* body) {
#ifdef _WIN32
    static int wsa_init = 0;
    if (!wsa_init) {
        WSADATA wsa;
        WSAStartup(MAKEWORD(2, 2), &wsa);
        wsa_init = 1;
    }
#endif

    orionis_sock_t sock = socket(AF_INET, SOCK_STREAM, 0);
    if (sock < 0) return;

    struct sockaddr_in serv_addr;
    memset(&serv_addr, 0, sizeof(serv_addr));
    serv_addr.sin_family = AF_INET;
    serv_addr.sin_port = htons(orionis_engine_port);
    serv_addr.sin_addr.s_addr = inet_addr(orionis_engine_host);

    if (connect(sock, (struct sockaddr*)&serv_addr, sizeof(serv_addr)) < 0) {
        ORIONIS_CLOSESOCKET(sock);
        return;
    }

    char header[512];
    int header_len = snprintf(header, sizeof(header),
        "POST /api/ingest HTTP/1.1\r\n"
        "Host: %s:%d\r\n"
        "Content-Type: application/json\r\n"
        "Content-Length: %zu\r\n"
        "Connection: close\r\n\r\n",
        orionis_engine_host, orionis_engine_port, strlen(body));

    send(sock, header, header_len, 0);
    send(sock, body, strlen(body), 0);
    ORIONIS_CLOSESOCKET(sock);
}

// ── API ──────────────────────────────────────────────────────────────────

static void orionis_emit(const char* type, const char* func, const char* file, int line, const char* span_id, const char* parent_id, uint64_t dur) {
    if (orionis_tid[0] == '\0') orionis_new_uuid(orionis_tid);

    char dur_str[32] = "null";
    if (dur > 0) snprintf(dur_str, sizeof(dur_str), "%llu", (unsigned long long)dur);

    char body[1024];
    snprintf(body, sizeof(body),
        "{\"trace_id\":\"%s\",\"span_id\":\"%s\",\"parent_span_id\":%s,"
        "\"timestamp_ms\":%llu,\"event_type\":\"%s\",\"function_name\":\"%s\","
        "\"module\":\"c_app\",\"file\":\"%s\",\"line\":%d,\"language\":\"c\","
        "\"duration_us\":%s}",
        orionis_tid, span_id, parent_id ? parent_id : "null", (unsigned long long)orionis_now_ms(),
        type, func, file, line, dur_str);

    char full_payload[1200];
    snprintf(full_payload, sizeof(full_payload), "[%s]", body);
    orionis_post(full_payload);
}

static void orionis_start(const char* url) {
    if (url) strncpy(orionis_engine_url, url, sizeof(orionis_engine_url)-1);
    srand(time(NULL));
    orionis_new_uuid(orionis_tid);
    printf("[Orionis] C Agent started: %s\n", orionis_engine_url);
}

static void orionis_stop() {
    orionis_tid[0] = '\0';
}

// ── Instrumentation ────────────────────────────────────────────────────────

typedef struct {
    char span_id[64];
    char func[128];
    char file[256];
    int  line;
    uint64_t start_ms;
} orionis_span_t;

static void orionis_enter(orionis_span_t* s, const char* func, const char* file, int line) {
    orionis_new_uuid(s->span_id);
    strncpy(s->func, func, sizeof(s->func)-1);
    strncpy(s->file, file, sizeof(s->file)-1);
    s->line = line;
    s->start_ms = orionis_now_ms();
    orionis_emit("function_enter", func, file, line, s->span_id, NULL, 0);
}

static void orionis_exit(orionis_span_t* s) {
    uint64_t dur = (orionis_now_ms() - s->start_ms) * 1000;
    char next_id[64];
    orionis_new_uuid(next_id);
    orionis_emit("function_exit", s->func, s->file, s->line, next_id, s->span_id, dur);
}

#if defined(__GNUC__) || defined(__clang__)
static void orionis_cleanup_span(orionis_span_t* s) {
    orionis_exit(s);
}
#define ORIONIS_TRACE() \
    orionis_span_t _orion_span __attribute__((cleanup(orionis_cleanup_span))); \
    orionis_enter(&_orion_span, __func__, __FILE__, __LINE__)
#else
// Fallback for non-GCC/Clang: requires manual enter/exit or block macros
#define ORIONIS_TRACE_BEGIN(name) \
    orionis_span_t _orion_span_##name; \
    orionis_enter(&_orion_span_##name, #name, __FILE__, __LINE__)
#define ORIONIS_TRACE_END(name) \
    orionis_exit(&_orion_span_##name)
#endif

#endif // ORIONIS_H
