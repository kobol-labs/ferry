# ⛴️ Ferry

> **Ultra-fast, headless hybrid LLM router and metrics proxy written in Rust.**  
> Built for orchestrating on-prem GPU hardware with seamless, zero-queue cloud spillover and transparent context condensation.

---

## Why Ferry?

When running autonomous multi-agent swarms (e.g., Strix, Cursor swarms, AutoGen) against local GPUs, three fundamental bottlenecks occur:
1. **Head-of-Line Queuing**: A single GPU with $N$ hardware slots cannot process $N+M$ parallel requests without queuing, which frequently trips client-side stream timeouts.
2. **Context Ballooning**: As multi-turn agents accumulate deep conversation histories (>65k tokens), local prompt prefill latency degrades and risks GPU VRAM exhaustion.
3. **Redundant History Bloat**: Past chain-of-thought `<think>` blocks and massive historical tool command outputs permanently inflate turns, wasting bandwidth and triggering unnecessary cloud spillovers.

**Ferry** solves this by acting as an intelligent OpenAI-compatible proxy between your clients and your models:
* **Transparent Context Condensation**: Automatically strips historical `<think>` reasoning blocks and truncates stale tool outputs from older turns in $<0.01\text{ ms}$, saving 40%–60% of tokens before routing.
* **Local Priority ($0)**: Keeps requests on your LAN GPU rig while hardware slots are available.
* **Instant Spillover**: The moment local slots are saturated, waiting requests **immediately spill over** to cloud providers (MiniMax, OpenRouter, Anthropic) with zero queuing lag.
* **Context Watermarking**: Prompts exceeding your local GPU's prefill threshold automatically route to cloud clusters.
* **Automatic Failover**: If a local stream terminates or times out, Ferry seamlessly retries the turn against the cloud fallback.
* **Observability**: Exposes real-time Prometheus `/metrics` and `/stats` to track local hardware savings, cloud spend, and condensation efficiency.
* **Network Isolation**: Can run with **zero exposed host ports**, communicating strictly across internal Docker bridge networks with client swarms.

---

## Empirical Benchmarks & Performance Profile

From a live 330+ turn penetration testing run using an NVIDIA RTX 3090 (Strata / Qwen3.8-Flash-Next) and Cloud MiniMax-M3:

| Provider | Context Scope | Count | Median Turn Time | p90 Turn Time | Fastest Turn |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Local Strata (RTX 3090)** | $\le$ 65,000 tokens (cached) | 35 turns | **2.59 seconds** | **5.73 seconds** | **1.74s** |
| **Cloud MiniMax-M3** | 70,000–140,000 tokens (heavy) | 295 turns | **5.82 seconds** | 49.38 seconds | **1.53s** |

---

## Configuration & Tuning Guide

### 1. Configuration File (`ferry.toml`)

```toml
[server]
bind = "0.0.0.0:4000"

[local]
name = "strata"
api_base = "http://192.168.9.99:8080/v1"
api_key = "sk-strata-stevegaming"
model = "qwen3.8-flash-next-iq2_xs"
max_slots = 3
max_context_tokens = 65000
timeout_secs = 90

[cloud]
name = "minimax"
api_base = "https://api.minimax.io/v1"
api_key = "sk-..."
model = "MiniMax-M3"
timeout_secs = 300

[condensation]
# Enable transparent prompt context condensation
enabled = true
# Strip verbose reasoning / <think> blocks from historical assistant messages
drop_historical_reasoning = true
# Truncate large tool / command outputs from older turns
truncate_tool_outputs = true
# Number of most recent turns to keep 100% byte-for-byte untouched (immediate context)
keep_recent_turns = 2
# Character threshold before an older tool output is truncated (default: 1500 chars)
max_tool_chars = 1500
# Lines to preserve from the head and tail of truncated tool outputs
head_lines = 15
tail_lines = 15
```

---

### 2. Tuning Parameters for Your Hardware

#### A. Calibrating Context Condensation (`[condensation]`)
- **`keep_recent_turns = 2`**: Preserves the last 2 assistant turns (and their tool outputs) completely untouched. The agent retains full, unedited immediate context.
- **`drop_historical_reasoning = true`**: Removes `<think>` scratchpads and `reasoning_content` from turns $\le N-2$. The agent never needs to re-read its own internal scratchpads from 5 turns ago to make current decisions.
- **`max_tool_chars = 1500`**: Any tool output in turns $\le N-2$ exceeding this limit is trimmed to `head_lines` and `tail_lines` with a clear notice (`[... Ferry Proxy: truncated 85 lines of earlier tool output ...]`).

#### B. Calibrating `max_slots`
* **RTX 3090 / 4090 (24GB VRAM)**:
  * With a quantized MoE model (e.g. Qwen3.8-Flash-Next at IQ2_XS taking ~13GB VRAM) and `int8` KV cache, setting `max_slots = 3` is the optimal balance. Each active slot reserves ~2–3GB for resident KV contexts.
* **Dual GPU (48GB VRAM) / Mac Studio (64GB+ unified memory)**:
  * Can safely increase `max_slots = 6` to `8`.

#### C. Calibrating `max_context_tokens` (The Spillover Watermark)
This parameter controls at what prompt size Ferry bypasses local hardware to avoid VRAM oversubscription:
* Combined with **Context Condensation**, prompt sizes rarely exceed 35k–45k tokens, allowing **85%–95% of requests to run locally on Strata** without exceeding the 65,000 token VRAM ceiling!

---

### 3. Environment Variable Overrides (Zero-Config Docker)

Ferry supports full zero-config deployment by overriding any setting via environment variables:

| Variable | Default | Description |
| :--- | :--- | :--- |
| `FERRY_BIND` | `0.0.0.0:4000` | Address and port to bind inside container |
| `FERRY_LOCAL_API_BASE` | `http://192.168.9.99:8080/v1` | Local inference engine base URL |
| `FERRY_LOCAL_API_KEY` | `sk-strata-stevegaming` | Local engine API key |
| `FERRY_LOCAL_MODEL` | `qwen3.8-flash-next-iq2_xs` | Local model name |
| `FERRY_LOCAL_MAX_SLOTS` | `3` | Maximum concurrent hardware slots |
| `FERRY_LOCAL_MAX_CONTEXT`| `65000` | Context watermark threshold |
| `FERRY_CLOUD_API_BASE` | `https://api.minimax.io/v1` | Cloud fallback base URL |
| `FERRY_CLOUD_API_KEY` | `""` | Cloud provider API key |
| `FERRY_CLOUD_MODEL` | `MiniMax-M3` | Cloud model name |
| `FERRY_CONDENSATION_ENABLED` | `true` | Enable/disable prompt context condensation |
| `FERRY_DROP_HISTORICAL_REASONING` | `true` | Strip past reasoning/<think> blocks |
| `FERRY_TRUNCATE_TOOL_OUTPUTS` | `true` | Truncate stale tool outputs |
| `FERRY_KEEP_RECENT_TURNS` | `2` | Number of recent turns preserved untouched |
| `FERRY_MAX_TOOL_CHARS` | `1500` | Maximum length of older tool outputs |

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

## Observability & Metrics

Ferry exposes built-in telemetry endpoints for real-time monitoring:

### 1. JSON Statistics (`GET /stats`)

```bash
curl -s http://127.0.0.1:4000/stats | jq
```

Example output:
```json
{
  "total_requests": 332,
  "local_requests": 142,
  "cloud_requests": 190,
  "spillover_requests": 48,
  "failover_requests": 0,
  "active_local_slots": 2,
  "total_tokens_est": 21250400,
  "local_tokens_est": 8185000,
  "cloud_tokens_est": 13065400,
  "local_ratio_percent": 42.77,
  "condensed_requests": 189,
  "chars_saved": 8452100,
  "reasoning_blocks_dropped": 340,
  "tool_outputs_truncated": 182
}
```

### 2. Prometheus Scrape Target (`GET /metrics`)

```text
# HELP ferry_requests_total Total number of LLM requests
# TYPE ferry_requests_total counter
ferry_requests_total 332
ferry_requests_total{provider="local"} 142
ferry_requests_total{provider="cloud"} 190
ferry_requests_total{type="spillover"} 48
ferry_requests_total{type="failover"} 0
# HELP ferry_condensed_requests_total Total requests optimized by condensation
# TYPE ferry_condensed_requests_total counter
ferry_condensed_requests_total 189
# HELP ferry_chars_saved_total Total characters pruned from prompts
# TYPE ferry_chars_saved_total counter
ferry_chars_saved_total 8452100
# HELP ferry_reasoning_blocks_dropped_total Total historical reasoning blocks stripped
# TYPE ferry_reasoning_blocks_dropped_total counter
ferry_reasoning_blocks_dropped_total 340
# HELP ferry_tool_outputs_truncated_total Total historical tool outputs truncated
# TYPE ferry_tool_outputs_truncated_total counter
ferry_tool_outputs_truncated_total 182
```

---

## Architecture Blueprint

```text
Client (Strix / Swarm)
         │
         ▼
┌────────────────────────────────────────────────────────┐
│  FERRY TRANSPARENT CONDENSER (< 0.01 ms)               │
│  - Strip reasoning blocks older than 2 turns           │
│  - Truncate stale tool outputs exceeding 1,500 chars   │
└────────────────────────────────────────────────────────┘
         │ (Condensed Payload)
         ▼
┌────────────────────────────────────────────────────────┐
│  HYBRID ROUTER                                         │
│  - Context <= 65k & Slots < 3 ──► Local Strata ($0)    │
│  - Context > 65k ───────────────► Cloud MiniMax Burst  │
│  - Slots Saturated (3/3) ───────► Cloud Zero-Queue     │
└────────────────────────────────────────────────────────┘
```

---

## License

MIT © [Kobol Labs](https://github.com/kobol-labs)
