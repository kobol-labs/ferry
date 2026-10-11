# ⛴️ Ferry

> **Ultra-fast, headless hybrid LLM router and metrics proxy written in Rust.**  
> Built for orchestrating on-prem GPU hardware with atomic slot scheduling, short contention queueing, seamless cloud spillover, automatic circuit breaking, and transparent context condensation.

---

## Why Ferry?

When running autonomous multi-agent swarms (e.g., Strix, Cursor agents, AutoGen) against local GPUs, three fundamental bottlenecks occur:
1. **Head-of-Line Queuing & Concurrency Races**: A single GPU with $N$ hardware slots cannot process $N+M$ parallel requests without queuing, which frequently trips client-side stream timeouts or exhausts GPU VRAM if overcommitted.
2. **Context Ballooning**: As multi-turn agents accumulate deep conversation histories (>65k tokens), local prompt prefill latency degrades and risks GPU out-of-memory (OOM) crashes.
3. **Redundant History Bloat**: Past chain-of-thought `<think>` blocks and massive historical tool command outputs permanently inflate turns, wasting bandwidth, degrading inference speeds, and triggering unnecessary cloud spend.

**Ferry** solves this by acting as an ultra-lean, OpenAI-compatible proxy between your clients and your models:
* **Atomic Slot Reservation & Leak-Proof Lifecycle**: Atomic Compare-And-Swap (CAS) ensures hardware slots are never oversubscribed. Full RAII guards guarantee slots are immediately released even if a client disconnects, times out, or cancels mid-stream.
* **Short Contention Queue**: Instead of immediately dumping bursts to expensive cloud APIs when all local slots are momentarily occupied, Ferry holds requests in a lightweight 20ms polling queue (e.g. up to 500ms) to capture freeing GPU slots locally.
* **Transparent Context Condensation**: Strips historical `<think>` reasoning blocks and safely truncates stale tool outputs from older turns in $<0.01\text{ ms}$, saving 30%–60% of tokens before routing.
* **Built-in Safety & Cache Invariants**:
  * **Static Prefix Alignment**: Preserves 100% byte-for-byte immutability across system prompts, developer messages, and initial user instructions to guarantee 90%+ KV cache hit rates on GPU and Cloud engines.
  * **Markdown Code-Block Protection**: Code blocks within tool outputs are automatically segmented and protected from destructive cleaners.
  * **Proof-of-Safety Constraint Ledger**: Verifies critical anchors (file paths, URLs, ports, IP addresses, CVEs, and security imperatives) survive compression; auto-vetoes compression on active turns if any anchor is damaged.
  * **Signal-Aware Truncation**: Automatically scans middle log sections for error signals, panics, and security findings (e.g. `fatal error:`, `panic:`, `cwe-`, `vulnerability`) to retain the diagnostic needle even when truncating massive outputs.
* **Runaway Repetition Folding**: Detects and folds duplicate historical tool failures across consecutive retry attempts (e.g. connection refused polling loops), preventing infinite prompt inflation while keeping the active turn untouched.
* **Terminal Noise Cleaner**: Consolidated high-speed cleaner stripping ANSI escape codes, carriage-return progress overwrites (`\r`), excess blank lines, and minifying pretty-printed JSON outputs.
* **Automatic Circuit Breaker & Health Probing**: Continuously health-probes local GPU endpoints in the background. If the local engine goes offline or fails consecutively, the circuit breaker trips open to route directly to Cloud with 0ms penalty, automatically resetting when local recovers.
* **Client Routing & Condensation Overrides**: Allows callers to selectively bypass condensation (`X-Ferry-Condense: false`) or enforce strict routing constraints (`X-Ferry-Route: local-only` with fast-fail 503 or `X-Ferry-Route: cloud-only`).
* **Deep Observability**: Returns operational metadata headers on every response (`X-Ferry-Provider`, `X-Ferry-Model`, `X-Ferry-Est-Tokens`, `X-Ferry-Chars-Saved`, `X-Ferry-Savings-Percent`, `X-Ferry-Duration-Ms`, `X-Ferry-Failover`) alongside `/health`, `/stats`, and Prometheus `/metrics`.
* **Minimal Resource Footprint**: Written in pure Rust with zero garbage collection overhead. Uses ~1.1 MB RAM idle, ~2.4 MB RAM active, <0.5% CPU, with a 7.2 MB compiled binary and 26 MB Alpine Docker container.

---

## Empirical Benchmarks & Performance Profile

From a full end-to-end autonomous penetration test run against Invoicely across 30+ attack surfaces using an NVIDIA RTX 3090 (Strata / Qwen3.8-Flash-Next) and Cloud MiniMax-M3:

| Metric | Result |
| :--- | :--- |
| **Total Requests Processed** | 406 requests |
| **Total Input Tokens Processed** | 32,150,000 tokens |
| **Tokens Saved by Ferry Condensation** | **8,482,000 tokens (26.4% context reduction)** |
| **KV Cache Hit Rate** | **92.5% (29,620,000 cached tokens)** |
| **Circuit Breaker Trips / Failures** | 0 trips (100% engine uptime) |
| **Median Local Turn Latency** | **2.59 seconds** |
| **Fastest Local Turn** | **1.53 seconds** |

---

## Configuration & Tuning Guide

### 1. Configuration File (`ferry.toml`)

```toml
[server]
bind = "0.0.0.0:4000"

[local]
api_base = "http://192.168.9.99:8080/v1"
api_key = "sk-strata-stevegaming"
model = "qwen3.8-flash-next-iq2_xs"
max_slots = 3
max_context_tokens = 65000
timeout_secs = 90
connect_timeout_secs = 3
circuit_breaker_threshold = 3
probe_interval_secs = 5
queue_timeout_ms = 500

[cloud]
api_base = "https://api.minimax.io/v1"
api_key = "sk-..."
model = "MiniMax-M3"
timeout_secs = 300

[condensation]
# Enable transparent prompt context condensation before routing
enabled = true
# Strip verbose reasoning / <think> blocks from historical assistant messages
drop_historical_reasoning = true
# Truncate large tool / command outputs from older turns
truncate_tool_outputs = true
# Number of most recent turns to keep 100% byte-for-byte untouched (immediate context)
keep_recent_turns = 1
# Character threshold before an older tool output is truncated (default: 2000 chars)
max_tool_chars = 2000
# Runaway safety cap on recent turn tool outputs to avoid blinding auditor on code files (default: 60000 chars)
recent_max_tool_chars = 60000
# Lines to preserve from the head and tail of truncated tool outputs
head_lines = 15
tail_lines = 10
# Streamlined cleaners & loop folding:
strip_terminal_noise = true          # Strip ANSI escapes, progress bar overwrites, excess blank lines
deduplicate_lines = true             # Collapse consecutive identical lines with repeat count
minify_json_outputs = true           # Minify pretty-printed JSON tool outputs (saves ~30%)
fold_repeated_tool_failures = true   # Fold runaway repetitive tool errors across historical turns

# Engine Invariants (always enforced, zero configuration needed):
# - Static Prefix Protection: 100% byte-for-byte immutable system/developer/user prompts for 90%+ KV cache hit rate
# - Markdown Code Block Protection: code blocks are 100% exempt from destructive cleaners
# - Signal-Aware Truncation: diagnostic errors/exceptions in middle of long logs are automatically detected and preserved
# - Constraint Ledger Verification: safety anchors (IPs, paths, CVEs, security imperatives) are verified with automatic rollback
```

---

### 2. Tuning Parameters for Your Hardware

#### A. Calibrating Context Condensation (`[condensation]`)
- **`keep_recent_turns = 1`**: Keeps the most recent turn (and its tool outputs) 100% byte-for-byte untouched so the model retains full immediate context.
- **`drop_historical_reasoning = true`**: Removes `<think>` scratchpads and `reasoning_content` from older historical turns. The agent never needs to re-read its own internal scratchpads from prior turns to make current decisions.
- **`max_tool_chars = 2000`**: Tool outputs in older turns exceeding this threshold are trimmed to `head_lines` and `tail_lines` with a clear omission notice (`[... Ferry Proxy: truncated 85 lines of earlier tool output ...]`). Middle diagnostic failure signals are automatically preserved.
- **`recent_max_tool_chars = 60000`**: Runaway safety cap on recent active tool turns to prevent giant terminal dumps from exhausting context while protecting code files under audit.
- **`strip_terminal_noise = true`**: Strips ANSI escape sequences, carriage-return progress overwrites, and collapses excess blank lines.
- **`fold_repeated_tool_failures = true`**: Automatically collapses runaway repetition loops (e.g. repeated connection refused or 404 responses across historical turns) into compact repeat counters.
- **Engine Invariants**: Prefix cache immutability (`system`/`developer`/`user`), code block shielding, constraint ledger anchor verification, and diagnostic signal preservation are permanent engine invariants requiring zero configuration.

#### B. Calibrating `max_slots` and `queue_timeout_ms`
* **RTX 3090 / 4090 (24GB VRAM)**:
  * With a quantized MoE model (e.g. Qwen3.8-Flash-Next at IQ2_XS taking ~13GB VRAM) and `int8` KV cache, setting `max_slots = 3` is the optimal balance. Each active slot reserves ~2–3GB for resident KV contexts.
* **Contention Queue (`queue_timeout_ms = 500`)**:
  * When all 3 slots are busy, turns that finish within 200–500ms will release their slot to waiting requests locally without incurring unnecessary cloud API billing.

#### C. Calibrating `circuit_breaker_threshold` & `probe_interval_secs`
* **`circuit_breaker_threshold = 3`**: If Strata fails 3 consecutive times (or probe connection fails), Ferry automatically trips the circuit breaker and routes traffic to Cloud with 0ms connection latency.
* **`probe_interval_secs = 5`**: Background health task queries `/models` on Strata every 5s and automatically resets the circuit breaker the instant local hardware recovers.

---

### 3. Client Control Headers & Observability

#### Client Request Headers
Clients can customize routing and condensation behavior on a per-request basis:

| Header | Values | Description |
| :--- | :--- | :--- |
| `X-Ferry-Condense` | `true` \| `false` | Bypasses prompt condensation when set to `false`, `0`, or `no`. |
| `X-Ferry-Route` | `local-only` \| `cloud-only` | `local-only` forces local hardware and returns HTTP 503 if unavailable (preventing cloud spend). `cloud-only` bypasses local hardware entirely. |

#### Observability Response Headers
Every response includes detailed routing metrics and CORS exposure headers:

```http
HTTP/1.1 200 OK
x-ferry-provider: local
x-ferry-model: qwen3.8-flash-next-iq2_xs
x-ferry-est-tokens: 24
x-ferry-chars-saved: 138
x-ferry-savings-percent: 34.94%
x-ferry-duration-ms: 822
x-ferry-failover: false
access-control-expose-headers: x-ferry-provider, x-ferry-model, x-ferry-est-tokens, x-ferry-chars-saved, x-ferry-savings-percent, x-ferry-duration-ms, x-ferry-failover
```

---

### 4. Environment Variable Overrides (Zero-Config Docker)

Ferry supports full zero-config deployment by overriding any setting via environment variables:

| Variable | Default | Description |
| :--- | :--- | :--- |
| `FERRY_BIND` | `0.0.0.0:4000` | Address and port to bind inside container |
| `FERRY_LOCAL_API_BASE` | `http://192.168.9.99:8080/v1` | Local inference engine base URL |
| `FERRY_LOCAL_API_KEY` | `sk-strata-stevegaming` | Local engine API key |
| `FERRY_LOCAL_MODEL` | `qwen3.8-flash-next-iq2_xs` | Local model name |
| `FERRY_LOCAL_MAX_SLOTS` | `3` | Maximum concurrent hardware slots |
| `FERRY_LOCAL_MAX_CONTEXT` | `65000` | Context watermark threshold in tokens |
| `FERRY_LOCAL_TIMEOUT_SECS` | `90` | Request timeout for local engine |
| `FERRY_LOCAL_CONNECT_TIMEOUT_SECS` | `3` | Fast TCP connect timeout for local engine |
| `FERRY_CIRCUIT_BREAKER_THRESHOLD` | `3` | Consecutive failures before tripping circuit breaker |
| `FERRY_PROBE_INTERVAL_SECS` | `5` | Background health probe interval in seconds |
| `FERRY_LOCAL_QUEUE_TIMEOUT_MS` | `500` | Slot contention queue timeout in milliseconds |
| `FERRY_CLOUD_API_BASE` | `https://api.minimax.io/v1` | Cloud fallback base URL |
| `FERRY_CLOUD_API_KEY` | `""` | Cloud provider API key |
| `FERRY_CLOUD_MODEL` | `MiniMax-M3` | Cloud model name |
| `FERRY_CLOUD_TIMEOUT_SECS` | `300` | Request timeout for cloud fallback |
| `FERRY_CONDENSATION_ENABLED` | `true` | Enable/disable prompt context condensation |
| `FERRY_DROP_HISTORICAL_REASONING` | `true` | Strip past reasoning / `<think>` blocks |
| `FERRY_TRUNCATE_TOOL_OUTPUTS` | `true` | Truncate stale tool outputs |
| `FERRY_KEEP_RECENT_TURNS` | `1` | Number of recent turns preserved untouched |
| `FERRY_MAX_TOOL_CHARS` | `2000` | Maximum length of older tool outputs |
| `FERRY_RECENT_MAX_TOOL_CHARS` | `60000` | Runaway safety cap on recent turn tool outputs |
| `FERRY_HEAD_LINES` | `15` | Head lines preserved in truncated tool output |
| `FERRY_TAIL_LINES` | `10` | Tail lines preserved in truncated tool output |
| `FERRY_STRIP_TERMINAL_NOISE` | `true` | Strip ANSI escapes, progress bar overwrites, excess blank lines (also accepts `FERRY_STRIP_ANSI`) |
| `FERRY_DEDUPLICATE_LINES` | `true` | Collapse runs of 3+ consecutive identical lines |
| `FERRY_MINIFY_JSON` | `true` | Minify JSON outputs |
| `FERRY_FOLD_REPEATED_TOOL_FAILURES` | `true` | Fold runaway repetitive tool errors across historical turns |

---

## Deployment Modes

### Mode 1: Dedicated Internal Network (Recommended for Security Swarms)

To ensure Ferry **only** accepts traffic from client agents (e.g., Strix) and exposes **no open ports on the host**, deploy via Docker Compose:

```yaml
services:
  ferry:
    image: ghcr.io/kobol-labs/ferry:latest
    container_name: ferry
    restart: unless-stopped
    volumes:
      - ./config/ferry.toml:/app/ferry.toml:ro
    expose:
      - "4000"  # Accessible inside strix-net only; NOT bound to host
    networks:
      - strix-net

  strix:
    image: ghcr.io/kobol-labs/strix:latest
    depends_on:
      ferry:
        condition: service_healthy
    environment:
      - OPENAI_BASE_URL=http://ferry:4000/v1
    networks:
      - strix-net

networks:
  strix-net:
    name: strix-net
```

---

## Observability & Telemetry Endpoints

### 1. Health Status (`GET /health`)

```bash
curl -s http://127.0.0.1:4000/health | jq
```

```json
{
  "status": "ok",
  "service": "ferry",
  "version": "0.4.0",
  "strata_online": true,
  "circuit_breaker_open": false,
  "consecutive_local_failures": 0,
  "circuit_trips_total": 0,
  "active_local_slots": 0,
  "max_local_slots": 3,
  "condensation_enabled": true
}
```

### 2. JSON Statistics (`GET /stats`)

```bash
curl -s http://127.0.0.1:4000/stats | jq
```

```json
{
  "total_requests": 406,
  "local_requests": 164,
  "cloud_requests": 242,
  "spillover_requests": 58,
  "failover_requests": 0,
  "active_local_slots": 0,
  "total_tokens_est": 32150000,
  "local_tokens_est": 9480000,
  "cloud_tokens_est": 22670000,
  "local_ratio_percent": 40.39,
  "condensed_requests": 242,
  "chars_saved": 29687000,
  "tokens_saved_est": 8482000,
  "reasoning_blocks_dropped": 412,
  "tool_outputs_truncated": 218,
  "ansi_stripped_count": 115,
  "progress_cleaned_count": 62,
  "lines_deduped_count": 145,
  "whitespace_collapsed_count": 260,
  "json_minified_count": 88,
  "ledger_checks_passed": 242,
  "ledger_vetoed_count": 0,
  "signals_extracted_count": 36,
  "loops_collapsed_count": 14,
  "strata_online": true,
  "circuit_breaker_open": false
}
```

### 3. Prometheus Scrape Target (`GET /metrics`)

```text
# HELP ferry_requests_total Total number of LLM requests
# TYPE ferry_requests_total counter
ferry_requests_total 332
ferry_requests_total{provider="local"} 142
ferry_requests_total{provider="cloud"} 190
ferry_requests_total{type="spillover"} 48
ferry_requests_total{type="failover"} 0

# HELP ferry_active_slots Current active local hardware slots
# TYPE ferry_active_slots gauge
ferry_active_slots 0

# HELP ferry_strata_online Health status of the local hardware provider (1 = online, 0 = offline)
# TYPE ferry_strata_online gauge
ferry_strata_online 1

# HELP ferry_circuit_open Whether the circuit breaker is currently open (1 = open, 0 = closed)
# TYPE ferry_circuit_open gauge
ferry_circuit_open 0

# HELP ferry_condensed_requests_total Total requests optimized by condensation
# TYPE ferry_condensed_requests_total counter
ferry_condensed_requests_total 189

# HELP ferry_chars_saved_total Total characters pruned from prompts
# TYPE ferry_chars_saved_total counter
ferry_chars_saved_total 8452100

# HELP ferry_tokens_saved_estimated_total Total tokens estimated saved
# TYPE ferry_tokens_saved_estimated_total counter
ferry_tokens_saved_estimated_total 2113025

# HELP ferry_request_duration_ms_total Total latency of completed LLM requests in milliseconds
# TYPE ferry_request_duration_ms_total counter
ferry_request_duration_ms_total{provider="local"} 95200
ferry_request_duration_ms_total{provider="cloud"} 1105400

# HELP ferry_last_request_duration_ms Latency of most recent completed request in milliseconds
# TYPE ferry_last_request_duration_ms gauge
ferry_last_request_duration_ms 822
```

### 4. Grafana Integration & Dashboard

A production-ready Grafana dashboard is included in [`grafana/dashboard.json`](file:///Users/steve/local/ferry/grafana/dashboard.json).

#### Prometheus Scrape Config
Add Ferry to your `prometheus.yml`:
```yaml
scrape_configs:
  - job_name: 'ferry'
    scrape_interval: 5s
    static_configs:
      - targets: ['192.168.9.99:4000'] # or your Ferry host/IP
```

#### Importing Dashboard
1. Open Grafana -> **Dashboards** -> **New** -> **Import**.
2. Upload `grafana/dashboard.json` or paste the JSON content.
3. Select your Prometheus data source.
4. Visualize live slot saturation, circuit breaker status, local vs. cloud latency curves, and token savings in real time.

---

## Alternative Use Cases

Beyond autonomous pentesting agents, Ferry is designed as a universal edge router for LLM workloads:

1. **Local Developer Co-pilot / Cursor Gateway**:
   Point Cursor, VS Code Continue, Claude Code, or Aider to `http://localhost:4000/v1`. Instant code completions run 100% free on local GPUs, while huge multi-file refactors seamlessly burst to Cloud APIs without reconfiguring your IDE.
2. **CI/CD Test Runner & Cost Protector**:
   Enforce zero unexpected cloud bills in automated test suites by injecting `X-Ferry-Route: local-only`. If local rigs are offline or busy, test runs fail fast with HTTP 503 instead of charging your credit card.
3. **RTK (Real-Time Terminal Cleaner) for Agent Swarms**:
   DevOps and automation swarms running noisy CLI tools (`nmap`, `docker`, `terraform`, `git`) output thousands of repetitive log lines, ANSI codes, and progress bars. Ferry transparently sanitizes and compacts these tool payloads on-the-fly, reducing downstream LLM prompt sizes by 40%–60%.
4. **Multi-Node Cluster Ingress**:
   Deploy Ferry as the front door for on-prem clusters (Ollama, vLLM, Strata/llama.cpp) across LAN servers, handling health checking, circuit breaking, and load balancing to Cloud providers.

---

## Architecture Blueprint

```text
Client (Strix / Agent Swarm / OpenWebUI)
         │
         │  [Optional Client Headers: X-Ferry-Condense, X-Ferry-Route]
         ▼
┌────────────────────────────────────────────────────────┐
│  FERRY TRANSPARENT CONDENSER (< 0.01 ms)               │
│  - Strip reasoning blocks older than 1 turn            │
│  - Truncate stale tool outputs (signal-aware preserve) │
│  - Terminal Noise Cleaner: ANSI, Progress, Whitespace  │
│  - Runaway Repetition Folding: collapse failure loops  │
│  - JSON Minifier: pretty JSON -> compact single line   │
│  - Constraint Ledger (Verify URLs, IPs, CVEs, Paths)   │
│  - Static Prefix Alignment (100% KV cache immutability)│
└────────────────────────────────────────────────────────┘
         │ (Optimized Wire Payload)
         ▼
┌────────────────────────────────────────────────────────┐
│  HYBRID ROUTER & CIRCUIT BREAKER                       │
│  - Circuit Breaker Open? ──────► Fast Cloud Failover   │
│  - Client 'cloud-only'? ───────► Cloud Direct          │
│  - Prompt Tokens > 65k? ───────► Cloud Direct          │
│  - Local Slots Available? ─────► Atomic Reserve & Run  │
│  - Slots Full? ────────────────► 500ms Contention Queue│
│    └─► Freed in <= 500ms? ─────► Local Slot Captured   │
│    └─► Still Full? ────────────► Cloud Spillover       │
│        (If 'local-only', returns 503 Service Unavail)  │
└────────────────────────────────────────────────────────┘
```

---

## License

MIT © [Kobol Labs](https://github.com/kobol-labs)
