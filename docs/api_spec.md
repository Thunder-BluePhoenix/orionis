# Orionis Stable API & Protocol Specification

## Ingestion API

### REST Ingestion
- **Endpoint**: `POST /api/ingest`
- **Auth**: `X-Orionis-Api-Key` header
- **Payload**: `TraceEvent` or `Vec<TraceEvent>` (JSON)

### gRPC Ingestion
- **Service**: `orionis.Ingest`
- **Method**: `StreamEvents (stream TraceEvent) returns (IngestResponse)`
- **Port**: `50051`

## Dashboard & Query API

### List Traces
- **Endpoint**: `GET /api/traces`
- **Auth**: `Bearer <JWT>`
- **Response**: `Vec<TraceSummary>`

### Get Trace Details
- **Endpoint**: `GET /api/traces/{id}`
- **Auth**: `Bearer <JWT>`

### Team Collaboration
- **Add Comment**: `POST /api/traces/{id}/comments`
- **Set Tags**: `POST /api/traces/{id}/tags`
- **Assign Trace**: `POST /api/traces/{id}/assign`

## Protocol Specification (v1.0)
Orionis uses a custom tracing protocol designed for high-throughput observability.

### Event Propagation
1. **Agent** captures local execution context.
2. **Agent** buffers and flushes events via gRPC or REST.
3. **Engine** receives event and calculates owning node via Consistent Hashing.
4. **Engine** forwards event (if not owner) to target node via gRPC.
5. **Storage** persists data with Tenant Isolation.

### Multi-Tenancy
All data includes a `tenant_id`. Access is enforced at the middleware layer using JWT claims or `X-Tenant-Id` headers.
