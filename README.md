# ⛴️ Ferry

> **Ultra-fast, headless hybrid LLM router and metrics proxy written in Rust.**  
> Built for orchestrating on-prem GPU hardware with seamless, zero-queue cloud spillover.

---

## Why Ferry?

When running autonomous multi-agent frameworks (e.g., Strix, Cursor swarms, AutoGen) against local GPUs, two fundamental bottlenecks occur:
1. **Head-of-Line Queuing**: A single GPU with $N$ hardware slots cannot process $N+M$ parallel requests without queuing, which frequently trips client-side stream timeouts.
2. **Context Ballooning**: As multi-turn agents accumulate large contexts (>65k tokens), local prompt prefill latency degrades, delaying subsequent turns.

**Ferry** solves this by acting as an OpenAI-compatible proxy between your clients and your models:
* **Local Priority ($0)**: Keeps requests on your LAN GPU rig while hardware slots are available.
* **Instant Spillover**: The moment local slots are saturated, waiting requests **immediately spill over** to cloud providers (MiniMax, OpenRouter, Anthropic) with zero queuing lag.
* **Context Watermarking**: Prompts exceeding your local GPU's prefill threshold automatically route to cloud clusters.
* **Automatic Failover**: If a local stream terminates or times out, Ferry seamlessly retries the turn against the cloud fallback.
* **Observability**: Exposes real-time Prometheus `/metrics` and `/stats` to track local hardware savings vs cloud spend.

---

## Quickstart

### 1. Configuration (`ferry.toml`)

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

### 2. Run with Cargo

```bash
cargo run --release
```

### 3. Run with Docker

```bash
docker build -t ghcr.io/kobol-labs/ferry:latest .
docker run -d --name ferry -p 4000:4000 -v $(pwd)/ferry.toml:/app/ferry.toml:ro ghcr.io/kobol-labs/ferry:latest
```

---

## API Endpoints

| Endpoint | Method | Description |
| :--- | :--- | :--- |
| `/v1/chat/completions` | `POST` | OpenAI-compatible chat completion proxy (supports streaming SSE) |
| `/v1/models` | `GET` | Lists available models and routing aliases |
| `/health` | `GET` | Service health status and active hardware slot counts |
| `/stats` | `GET` | JSON snapshot of request counts, token estimates, and local savings |
| `/metrics` | `GET` | Prometheus metrics scrape target |

---

## Metrics Tracked

* `ferry_requests_total{provider="local|cloud"}`
* `ferry_requests_total{type="spillover|failover"}`
* `ferry_active_slots`
* `ferry_tokens_estimated{provider="local|cloud"}`

---

## License

MIT © [Kobol Labs](https://github.com/kobol-labs)
