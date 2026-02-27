# Orionis Cluster Test — Fixed
# Fixes Node 2 connectivity: uses Set-Location inside job, explicit process launch,
# and retry-based health checks instead of fixed wait.

param(
    [string]$EnginePath = "f:\proj_guides\Orionis\orionis-engine",
    [string]$DataRoot = "f:\proj_guides\Orionis"
)

$Data1 = Join-Path $DataRoot "orionis-data-1"
$Data2 = Join-Path $DataRoot "orionis-data-2"
$BinaryPath = Join-Path $EnginePath "target\release\orionis-engine.exe"

# ── Build first (once, shared binary) ───────────────────────────────────────
Write-Host "`n[Orionis] Building engine (release mode)..." -ForegroundColor Cyan
Push-Location $EnginePath
cargo build --release
if ($LASTEXITCODE -ne 0) {
    Write-Host "[ERROR] Build failed. Aborting cluster test." -ForegroundColor Red
    Pop-Location
    exit 1
}
Pop-Location
Write-Host "[Orionis] Build OK: $BinaryPath" -ForegroundColor Green

# ── Cleanup old data ─────────────────────────────────────────────────────────
Remove-Item -Path $Data1 -Force -Recurse -ErrorAction SilentlyContinue
Remove-Item -Path $Data2 -Force -Recurse -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $Data1 -Force | Out-Null
New-Item -ItemType Directory -Path $Data2 -Force | Out-Null

# ── Launch Node 1 ────────────────────────────────────────────────────────────
Write-Host "`n[Orionis] Starting Node 1 on ports 7700 (HTTP) / 7701 (gRPC)..." -ForegroundColor Cyan
$n1 = Start-Process -FilePath $BinaryPath `
    -WorkingDirectory $DataRoot `
    -WindowStyle Normal `
    -PassThru `
    -Environment @{
    ORIONIS_DB_PATH   = $Data1
    ORIONIS_HTTP_ADDR = "0.0.0.0:7700"
    ORIONIS_GRPC_ADDR = "0.0.0.0:7701"
}
Write-Host "[Orionis] Node 1 PID: $($n1.Id)"

# ── Launch Node 2 ────────────────────────────────────────────────────────────
Write-Host "[Orionis] Starting Node 2 on ports 7800 (HTTP) / 7801 (gRPC)..." -ForegroundColor Cyan
$n2 = Start-Process -FilePath $BinaryPath `
    -WorkingDirectory $DataRoot `
    -WindowStyle Normal `
    -PassThru `
    -Environment @{
    ORIONIS_DB_PATH   = $Data2
    ORIONIS_HTTP_ADDR = "0.0.0.0:7800"
    ORIONIS_GRPC_ADDR = "0.0.0.0:7801"
}
Write-Host "[Orionis] Node 2 PID: $($n2.Id)"

# ── Wait for both nodes to be ready (retry-based, not fixed wait) ─────────────
function Wait-Node {
    param([string]$Url, [string]$Label, [int]$MaxRetries = 30, [int]$DelayMs = 1000)
    Write-Host "[Orionis] Waiting for $Label to be ready..." -ForegroundColor Yellow
    for ($i = 1; $i -le $MaxRetries; $i++) {
        try {
            $r = Invoke-WebRequest -Uri $Url -TimeoutSec 2 -ErrorAction Stop
            Write-Host "[Orionis] $Label is UP (attempt $i)" -ForegroundColor Green
            return $true
        }
        catch {
            Write-Host "  ... attempt $i/$MaxRetries" -ForegroundColor DarkGray
            Start-Sleep -Milliseconds $DelayMs
        }
    }
    Write-Host "[ERROR] $Label did not come up after $MaxRetries attempts!" -ForegroundColor Red
    return $false
}

$n1Ready = Wait-Node -Url "http://127.0.0.1:7700/api/nodes" -Label "Node 1 (7700)"
$n2Ready = Wait-Node -Url "http://127.0.0.1:7800/api/nodes" -Label "Node 2 (7800)"

if (-not $n1Ready -or -not $n2Ready) {
    Write-Host "`n[Orionis] Cluster startup failed. Stopping processes." -ForegroundColor Red
    Stop-Process -Id $n1.Id -Force -ErrorAction SilentlyContinue
    Stop-Process -Id $n2.Id -Force -ErrorAction SilentlyContinue
    exit 1
}

# ── Give nodes a moment to discover each other (heartbeat is every 5s) ────────
Write-Host "`n[Orionis] Waiting 8s for node heartbeat discovery..." -ForegroundColor Yellow
Start-Sleep -Seconds 8

# ── Verify node discovery ─────────────────────────────────────────────────────
Write-Host "`n[Orionis] === Nodes registered on Node 1 ===" -ForegroundColor Cyan
try {
    $r1 = Invoke-RestMethod -Uri "http://127.0.0.1:7700/api/nodes"
    $r1 | ConvertTo-Json -Depth 5
}
catch {
    Write-Host "[ERROR] Could not reach Node 1: $_" -ForegroundColor Red
}

Write-Host "`n[Orionis] === Nodes registered on Node 2 ===" -ForegroundColor Cyan
try {
    $r2 = Invoke-RestMethod -Uri "http://127.0.0.1:7800/api/nodes"
    $r2 | ConvertTo-Json -Depth 5
}
catch {
    Write-Host "[ERROR] Could not reach Node 2: $_" -ForegroundColor Red
}

# ── Test event forwarding: send trace to Node 1, verify it owns/forwards ──────
Write-Host "`n[Orionis] Testing event ingestion on Node 1..." -ForegroundColor Cyan
$testEvent = @{
    trace_id       = [System.Guid]::NewGuid().ToString()
    span_id        = [System.Guid]::NewGuid().ToString()
    parent_span_id = $null
    timestamp_ms   = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    event_type     = "function_enter"
    function_name  = "cluster_test"
    module         = "test"
    file           = "test_cluster.ps1"
    line           = 0
    language       = "powershell"
    duration_us    = $null
} | ConvertTo-Json

try {
    Invoke-RestMethod -Uri "http://127.0.0.1:7700/api/ingest" `
        -Method POST `
        -ContentType "application/json" `
        -Body "[$testEvent]"
    Write-Host "[OK] Ingestion to Node 1 succeeded" -ForegroundColor Green
}
catch {
    Write-Host "[ERROR] Ingestion failed: $_" -ForegroundColor Red
}

# ── Summary ──────────────────────────────────────────────────────────────────
Write-Host "`n============================================" -ForegroundColor Cyan
Write-Host " Orionis Cluster Test Complete" -ForegroundColor Cyan
Write-Host " Node 1: http://127.0.0.1:7700  (PID $($n1.Id))" -ForegroundColor Green
Write-Host " Node 2: http://127.0.0.1:7800  (PID $($n2.Id))" -ForegroundColor Green
Write-Host " Dashboards:" -ForegroundColor Cyan
Write-Host "   Node 1: http://127.0.0.1:7700" -ForegroundColor White
Write-Host "   Node 2: http://127.0.0.1:7800" -ForegroundColor White
Write-Host "============================================`n" -ForegroundColor Cyan
Write-Host "Press Enter to stop both nodes and exit..."
$null = Read-Host
Stop-Process -Id $n1.Id -Force -ErrorAction SilentlyContinue
Stop-Process -Id $n2.Id -Force -ErrorAction SilentlyContinue
Write-Host "[Orionis] Both nodes stopped." -ForegroundColor Yellow
