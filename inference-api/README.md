# inference-api

Production-ready inference runtime for `vLLM` on NVIDIA GPUs (target: 32 GB VRAM) with OpenAI-compatible endpoints (`/v1/*`) for the custom Codex fork.

Primary model target:

- `QuantTrio/Qwen3-VL-32B-Instruct-AWQ`

## Architecture (Vast Jupyter mode, recommended)

This image is intentionally "runtime-only":

- it does **not** manage Jupyter or SSH services inside the container;
- it runs vLLM only;
- in Vast launch mode `Jupyter-python notebook + SSH`, Vast keeps the instance alive with its own Jupyter process;
- vLLM is started as a secondary service via On-start Script or from Jupyter terminal.

This avoids restart loops when vLLM crashes: Jupyter remains available and you can inspect/fix directly.

## What changed in this "pro" setup

- Removed in-container Jupyter/SSH bootstrap logic.
- Kept a clean inference runtime based on `vllm/vllm-openai:v0.11.0`.
- Added `--daemon` mode to `start-vllm.sh`:
  - starts vLLM in background;
  - writes PID and logs to files;
  - returns immediately (good for Vast On-start Script).
- Kept foreground mode for raw Docker usage.

## Files

- `Dockerfile`: runtime image (vLLM + tooling).
- `start-vllm.sh`: vLLM startup, memory tuning, URL printing, daemon mode.
- `docker-compose.yaml`: local compose stack.
- `.env.example`: suggested variables.

## Build

From repo root:

```bash
docker build -t qwen3-vl-awq-vllm:latest ./inference-api
```

Or with your Docker Hub tag:

```bash
docker build -t beltromatti/qwen3-vl-awq-vllm:latest ./inference-api
```

## Runtime modes

### 1) Raw Docker (foreground)

```bash
docker run --gpus all --rm -it \
  -p 8000:8000 \
  -v "$HOME/.cache/huggingface:/data/huggingface" \
  -e HUGGING_FACE_HUB_TOKEN="hf_xxx" \
  beltromatti/qwen3-vl-awq-vllm:latest
```

Foreground mode is default (`ENTRYPOINT ["/opt/start-vllm.sh"]`).

### 2) Vast Jupyter mode (recommended)

In Vast:

- Launch mode: `Jupyter-python notebook + SSH`
- Optional: enable `Use Jupyter Lab interface`
- Ports: expose only container port `8000/TCP` for vLLM API
- Docker options (recommended):

```text
--ipc=host --shm-size=2g -v /workspace/huggingface:/data/huggingface
```

- On-start Script:

```bash
bash -lc "/opt/start-vllm.sh --daemon"
```

## Vast environment variables (safe start profile)

Batch paste recommended:

```text
HUGGING_FACE_HUB_TOKEN=hf_xxx
GPU_MEMORY_UTILIZATION=0.90
MAX_MODEL_LEN=24576
MAX_NUM_SEQS=1
SWAP_SPACE_GB=24
KV_CACHE_DTYPE=auto
ENABLE_KV_DTYPE_FALLBACK=0
LIMIT_MM_PER_PROMPT={"image":1,"video":0}
MM_PROCESSOR_CACHE_GB=0
SKIP_MM_PROFILING=1
ENFORCE_MM_DEFAULT_LIMITS=1
MM_DEFAULT_IMAGE_LIMIT=1
MM_DEFAULT_VIDEO_LIMIT=0
ENABLE_LOG_REQUESTS=0
```

Then increase progressively when stable.

## Daemon mode details

When called with `--daemon` or `--background`:

- log file: `${VLLM_LOG_FILE:-/workspace/vllm.log}`
- pid file: `${VLLM_PID_FILE:-/workspace/vllm.pid}`

Useful commands in Jupyter terminal:

```bash
tail -n 200 -f /workspace/vllm.log
cat /workspace/vllm.pid
kill "$(cat /workspace/vllm.pid)"
```

## Endpoint to use in Codex

Use the public mapped URL with `/v1` suffix:

```text
http://<PUBLIC_IP>:<EXTERNAL_PORT_FOR_8000>/v1
```

`start-vllm.sh` also prints a Codex-ready URL at startup.

## Memory model and context auto-sizing

If `MAX_MODEL_LEN` is not manually set, startup computes a safe/aggressive context length from:

- detected total GPU memory;
- `GPU_MEMORY_UTILIZATION`;
- weight/overhead budget parameters;
- KV cache byte cost per token (`fp8` vs non-fp8);
- hard cap, reserve and rounding.

Main knobs:

- `GPU_MEMORY_UTILIZATION` (default `0.95`)
- `KV_CACHE_DTYPE` (default `fp8`)
- `KV_FALLBACK_DTYPE` (default `auto`)
- `ENABLE_KV_DTYPE_FALLBACK` (default `1`)
- `MAX_MODEL_LEN` (optional manual override)
- `MAX_NUM_SEQS` (default `1`)
- `SWAP_SPACE_GB` (default `16`)
- `ENABLE_LOG_REQUESTS` (default `0`, set `1` to pass `--enable-log-requests`)
- `LIMIT_MM_PER_PROMPT` (preferred JSON string, e.g. `{"image":1,"video":0}`; legacy `image=1,video=0` is auto-converted)
- `MM_PROCESSOR_CACHE_GB` (default `0`, disables MM preprocessor cache to reduce RAM pressure)
- `SKIP_MM_PROFILING` (default `1`, avoids heavy MM startup profiling allocations)
- `ENFORCE_MM_DEFAULT_LIMITS` (default `1`, auto-fills missing MM limits)
- `MM_DEFAULT_IMAGE_LIMIT` (default `1`)
- `MM_DEFAULT_VIDEO_LIMIT` (default `0`)
- `ENABLE_STARTUP_CONTEXT_FALLBACK` (default `1`, retry with lower context if startup fails)
- `CONTEXT_FALLBACK_STEP` (default `4096`)
- `MIN_STARTUP_MAX_MODEL_LEN` (default `24576`)

Note for daemon mode:

- if `KV_CACHE_DTYPE=fp8` and fallback is enabled, script switches to fallback dtype before launch (recommended Vast profile remains `KV_CACHE_DTYPE=auto`, `ENABLE_KV_DTYPE_FALLBACK=0`).

## Healthcheck

Container healthcheck:

```text
GET http://127.0.0.1:${PORT:-8000}/health
```

Manual check:

```bash
curl -s http://localhost:8000/health
```

## Codex integration workflow

In the custom Codex build:

1. select model `Qwen3-VL-32B-Instruct-AWQ`
2. when prompted, paste your vLLM server URL (`.../v1`)
3. Codex stores provider `qwen_vllm` and uses this endpoint for requests

## Troubleshooting

If you hit OOM:

1. lower `MAX_MODEL_LEN`
2. lower `GPU_MEMORY_UTILIZATION`
3. reduce multimodal pressure (`LIMIT_MM_PER_PROMPT`)
4. keep `MAX_NUM_SEQS=1`
5. if you need text-only stability, set `LIMIT_MM_PER_PROMPT={"image":0,"video":0}` (this disables image/video input)

If startup fails in Vast:

- verify Hugging Face token;
- verify port 8000 exposure;
- inspect `/workspace/vllm.log`;
- rerun manually from Jupyter terminal.
- if failure is OOM, startup fallback automatically retries with lower `max_model_len` until `MIN_STARTUP_MAX_MODEL_LEN`.

## Security notes

- never commit HF tokens;
- prefer scoped/rotated tokens;
- treat public vLLM endpoints as sensitive and firewall when possible.
