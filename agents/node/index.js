const http = require('http');
const crypto = require('crypto');
const { AsyncLocalStorage } = require('async_hooks');

/**
 * Orionis Node.js Agent — Async-aware execution tracing.
 */
class Orionis {
    constructor() {
        this.engineUrl = 'http://localhost:7700';
        this.als = new AsyncLocalStorage();
        this.batch = [];
        this.flushInterval = setInterval(() => this.flush(), 100);
    }

    start(url) {
        if (url) this.engineUrl = url;
        console.log(`[Orionis] Node.js Agent started: ${this.engineUrl}`);
    }

    resetTrace() {
        const tid = crypto.randomUUID();
        this.als.enterWith({ traceId: tid });
        return tid;
    }

    getTraceId() {
        const store = this.als.getStore();
        if (store && store.traceId) return store.traceId;
        return this.resetTrace();
    }

    injectTraceHeaders(headers) {
        const tid = this.getTraceId().replace(/-/g, '');
        headers['traceparent'] = `00-${tid}-0000000000000000-01`;
    }

    extractTraceHeaders(headers) {
        const tp = headers['traceparent'];
        if (tp && tp.length >= 55) {
            const tidRaw = tp.substring(3, 35);
            const tid = `${tidRaw.substring(0, 8)}-${tidRaw.substring(8, 12)}-${tidRaw.substring(12, 16)}-${tidRaw.substring(16, 20)}-${tidRaw.substring(20, 32)}`;
            this.als.enterWith({ traceId: tid });
        }
    }

    emit(type, func, file, line, spanId, parentId, durUs) {
        const ev = {
            trace_id: this.getTraceId(),
            span_id: spanId,
            parent_span_id: parentId || null,
            timestamp_ms: Date.now(),
            event_type: type,
            function_name: func,
            module: 'node_app',
            file: file,
            line: line,
            language: 'node',
            duration_us: durUs || null,
            thread_id: 'main'
        };
        this.batch.push(ev);
    }

    flush() {
        if (this.batch.length === 0) return;

        const events = [...this.batch];
        this.batch = [];

        const body = JSON.stringify(events);
        const url = new URL(`${this.engineUrl}/api/ingest`);

        const req = http.request({
            hostname: url.hostname,
            port: url.port || 80,
            path: url.pathname,
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'Content-Length': Buffer.byteLength(body)
            }
        });

        req.on('error', (err) => {
            console.error(`[Orionis] Ingestion failed: ${err.message}`);
        });
        req.write(body);
        req.end();
    }

    /**
     * Wrap a function with tracing.
     */
    trace(name, fn) {
        const spanId = crypto.randomUUID();
        const start = process.hrtime.bigint();

        this.emit('function_enter', name, 'unknown', 0, spanId, null);

        return this.als.run(this.als.getStore() || { traceId: this.getTraceId() }, () => {
            try {
                const result = fn();
                // If result is a promise, wait for it
                if (result && typeof result.then === 'function') {
                    return result.then(res => {
                        const dur = Number(process.hrtime.bigint() - start) / 1000;
                        this.emit('function_exit', name, 'unknown', 0, crypto.randomUUID(), spanId, Math.round(dur));
                        return res;
                    }).catch(err => {
                        this.emit('exception', name, 'unknown', 0, crypto.randomUUID(), spanId, null);
                        throw err;
                    });
                }
                const dur = Number(process.hrtime.bigint() - start) / 1000;
                this.emit('function_exit', name, 'unknown', 0, crypto.randomUUID(), spanId, Math.round(dur));
                return result;
            } catch (err) {
                this.emit('exception', name, 'unknown', 0, crypto.randomUUID(), spanId, null);
                throw err;
            }
        });
    }
}

module.exports = new Orionis();
