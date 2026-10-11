# ⛴️ Ferry

> **Ultra-fast, headless hybrid LLM router and metrics proxy written in Rust.**  
> Built for orchestrating on-prem GPU hardware with seamless, zero-queue cloud spillover.

---

## Why Ferry?

When running autonomous multi-agent swarms (e.g., Strix, Cursor swarms, AutoGen) against local GPUs, two fundamental bottlenecks occur:
1. **Head-of-Line Queuing**: A single GPU with $N$ hardware slots cannot process $N+M$ parallel requests without queuing, which frequently trips client-side stream timeouts.
2. **Context Ballooning**: As multi-turn agents accumulate large contexts (>65k tokens), local prompt prefill latency degrades, delaying subsequent turns.

**Ferry** solves this by acting as an OpenAI-compatible proxy between your clients and your models:
* **Local Priority ($0)**: Keeps requests on your LAN GPU rig while hardware slots are available.
* **Instant Spillover**: The moment local slots are saturated, waiting requests **immediately spill over** to cloud providers (MiniMax, OpenRouter, Anthropic) with zero queuing lag.
* **Context Watermarking**: Prompts exceeding your local GPU's prefill threshold automatically route to cloud clusters.
* **Automatic Failover**: If a local stream terminates or times out, Ferry seamlessly retries the turn against the cloud fallback.
* **Observability**: Exposes real-time Prometheus `/metrics` and `/stats` to track local hardware savings vs cloud spend.
* **Network Isolation**: Can run with **zero exposed host ports**, communicating strictly across internal Docker bridge networks with client swarms.

---

## Empirical Benchmarks & Performance Profile

From a live 330+ turn penetration testing run using an NVIDIA RTX 3090 (Strata / Qwen3.8-Flash-Next) and Cloud MiniMax-M3:

| Provider | Context Scope | Count | Median Turn Time | p90 Turn Time | Fastest Turn |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Local Strata (RTX 3090)** | $\le$ 65,000 tokens (cached) | 35 turns | **2.59 seconds** | **5.73 seconds** | **1.74s** |
| **Cloud MiniMax-M3** | 70,000–140,000 tokens (heavy) | 295 turns | **5.82 seconds** | 49.38 seconds | **1.53s** |

### Key Takeaways
1. **Local is 2x Faster on Steady-State Turns**: With sub-millisecond LAN latency and GPU KV-cache prefix reuse, local inference outperforms cloud round-trips for medium contexts.
2. **Cloud Datacenters Win on Massive Prefill**: Evaluating 100k+ tokens of cold prompt over single-GPU PCIe takes 40–60s on consumer hardware, but completes in **1.5–3.0s** on cloud clusters.
3. **Quota Protection**: By delegating steady-state turns to local hardware, an intensive 32-million-token audit consumed only **14% of the 5-hour cloud interval quota**.

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
```

---

### 2. Tuning Parameters for Your Hardware

#### A. Calibrating `max_slots`
* **RTX 3090 / 4090 (24GB VRAM)**:
  * With a quantized MoE model (e.g. Qwen3.8-Flash-Next at IQ2_XS taking ~13GB VRAM) and `int8` KV cache, setting `max_slots = 3` is the optimal balance. Each active slot reserves ~2–3GB for resident KV contexts.
* **Dual GPU (48GB VRAM) / Mac Studio (64GB+ unified memory)**:
  * Can safely increase `max_slots = 6` to `8`.
* **Single 16GB GPU (RTX 4080 / 3080)**:
  * Keep `max_slots = 1` or `2` to prevent out-of-memory errors.

#### B. Calibrating `max_context_tokens` (The Spillover Watermark)
This parameter controls at what prompt size Ferry bypasses local hardware to avoid PCIe prefill delays:

* **High Local Offload (`max_context_tokens = 65000`)**:
  * *Best for*: Conserving cloud API quota and maximizing on-prem utilization.
  * *Behavior*: Keeps all turns up to 65k tokens local. Prompts >65k spill over to cloud.
* **Balanced Speed & Quota Burn (`max_context_tokens = 35000` to `40000`)**:
  * *Best for*: When you have unused interval credits (e.g., 5-hour cloud resets) and want faster prefill across large turns.
  * *Behavior*: Cuts local prefill times to $<15\text{s}$; any deep context agent immediately runs at cloud cluster speed.
* **Aggressive Cloud Burst (`max_context_tokens = 15000`)**:
  * *Best for*: Maximum execution speed during intensive scans where cloud tokens are abundant.

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
      - ferry
    environment:
      - OPENAI_BASE_URL=http://ferry:4000/v1
    networks:
      - strix-net

networks:
  strix-net:
    name: strix-net
```

### Mode 2: Published Host Port (Standalone Docker)

If clients on your host machine or LAN need to route through Ferry:

```bash
docker run -d \
  --name ferry \
  --restart unless-stopped \
  -p 4000:4000 \
  -e FERRY_LOCAL_API_BASE="http://192.168.9.99:8080/v1" \
  -e FERRY_CLOUD_API_KEY="sk-cp-..." \
  ghcr.io/kobol-labs/ferry:latest
```

### Mode 3: Native Binary

```bash
cargo build --release
./target/release/ferry
```

---

## Observability & Metrics

Ferry exposes built-in telemetry endpoints for real-time monitoring:

### 1. JSON Statistics (`GET /stats`)

From host (when running in Docker Compose with no host ports):
```bash
docker compose exec ferry wget -qO- http://127.0.0.1:4000/stats | jq
```

From localhost (when running standalone on port 4000):
```bash
curl -s http://127.0.0.1:4000/stats | jq
```

Example output:
```json
{
  "total_requests": 332,
  "local_requests": 36,
  "cloud_requests": 296,
  "spillover_requests": 84,
  "failover_requests": 2,
  "active_local_slots": 0,
  "total_tokens_est": 31250400,
  "local_tokens_est": 2185000,
  "cloud_tokens_est": 29065400,
  "local_ratio_percent": 10.84
}
```

### 2. Prometheus Scrape Target (`GET /metrics`)

```bash
docker compose exec ferry wget -qO- http://127.0.0.1:4000/metrics
```

```text
# HELP ferry_requests_total Total number of LLM requests
# TYPE ferry_requests_total counter
ferry_requests_total 332
ferry_requests_total{provider="local"} 36
ferry_requests_total{provider="cloud"} 296
ferry_requests_total{type="spillover"} 84
ferry_requests_total{type="failover"} 2
# HELP ferry_active_slots Current active local hardware slots
# TYPE ferry_active_slots gauge
ferry_active_slots 0
# HELP ferry_tokens_estimated Total estimated tokens processed
# TYPE ferry_tokens_estimated counter
ferry_tokens_estimated 31250400
ferry_tokens_estimated{provider="local"} 2185000
ferry_tokens_estimated{provider="cloud"} 29065400
```

---

## Architecture Blueprint

```text
                           ┌──► [Context <= MaxTokens & Slots < 3] ──► Local Strata GPU ($0, <2.5s)
Client (Strix / Cursor) ───┼──► [Context > MaxTokens] ───────────────► Cloud MiniMax (Fast Prefill)
                           └──► [Local Slots Saturated (3/3)] ───────► Cloud MiniMax (Zero Queue Lag)
```

---

## License

MIT © [Kobol Labs](https://github.com/kobol-labs)
