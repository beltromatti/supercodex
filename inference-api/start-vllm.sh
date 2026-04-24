#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-foreground}"
if [[ "${MODE}" == "--daemon" || "${MODE}" == "--background" ]]; then
  VLLM_DAEMON=1
else
  VLLM_DAEMON=0
fi

LOG_FILE="${VLLM_LOG_FILE:-/workspace/vllm.log}"
PID_FILE="${VLLM_PID_FILE:-/workspace/vllm.pid}"
SELF_SCRIPT="$(readlink -f "$0" 2>/dev/null || printf '%s' "$0")"

MODEL_ID="${MODEL_ID:-QuantTrio/Qwen3-VL-32B-Instruct-AWQ}"
SERVED_MODEL_NAME="${SERVED_MODEL_NAME:-qwen3-vl-32b-instruct-awq}"
HOST="${HOST:-0.0.0.0}"
PORT="${PORT:-8000}"

GPU_MEMORY_UTILIZATION="${GPU_MEMORY_UTILIZATION:-0.95}"
SWAP_SPACE_GB="${SWAP_SPACE_GB:-16}"
TENSOR_PARALLEL_SIZE="${TENSOR_PARALLEL_SIZE:-1}"
MAX_NUM_SEQS="${MAX_NUM_SEQS:-1}"
MAX_NUM_BATCHED_TOKENS="${MAX_NUM_BATCHED_TOKENS:-0}"
LIMIT_MM_PER_PROMPT="${LIMIT_MM_PER_PROMPT:-image=1,video=0}"
ENABLE_LOG_REQUESTS="${ENABLE_LOG_REQUESTS:-0}"
MM_PROCESSOR_CACHE_GB="${MM_PROCESSOR_CACHE_GB:-0}"
SKIP_MM_PROFILING="${SKIP_MM_PROFILING:-1}"
MM_DEFAULT_IMAGE_LIMIT="${MM_DEFAULT_IMAGE_LIMIT:-1}"
MM_DEFAULT_VIDEO_LIMIT="${MM_DEFAULT_VIDEO_LIMIT:-0}"
ENFORCE_MM_DEFAULT_LIMITS="${ENFORCE_MM_DEFAULT_LIMITS:-1}"
ENABLE_STARTUP_CONTEXT_FALLBACK="${ENABLE_STARTUP_CONTEXT_FALLBACK:-1}"
CONTEXT_FALLBACK_STEP="${CONTEXT_FALLBACK_STEP:-4096}"
MIN_STARTUP_MAX_MODEL_LEN="${MIN_STARTUP_MAX_MODEL_LEN:-24576}"

# KV/cache tuning defaults tuned for 32GB cards with AWQ 32B.
KV_CACHE_DTYPE="${KV_CACHE_DTYPE:-fp8}"
KV_FALLBACK_DTYPE="${KV_FALLBACK_DTYPE:-auto}"
ENABLE_KV_DTYPE_FALLBACK="${ENABLE_KV_DTYPE_FALLBACK:-1}"

# Memory model used to derive a safe but aggressive max context.
MODEL_WEIGHTS_GIB="${MODEL_WEIGHTS_GIB:-20.0}"
RUNTIME_OVERHEAD_GIB="${RUNTIME_OVERHEAD_GIB:-4.0}"
MAX_CONTEXT_HARD_CAP="${MAX_CONTEXT_HARD_CAP:-57344}"
MIN_CONTEXT_LEN="${MIN_CONTEXT_LEN:-8192}"
CONTEXT_RESERVE_TOKENS="${CONTEXT_RESERVE_TOKENS:-2048}"
CONTEXT_ROUNDING_STEP="${CONTEXT_ROUNDING_STEP:-1024}"

ENABLE_AUTO_TOOL_CHOICE="${ENABLE_AUTO_TOOL_CHOICE:-1}"
TOOL_CALL_PARSER="${TOOL_CALL_PARSER:-hermes}"

HF_TOKEN="${HF_TOKEN:-${HUGGING_FACE_HUB_TOKEN:-${HF_API_TOKEN:-}}}"
if [[ -n "${HF_TOKEN}" ]]; then
  export HUGGING_FACE_HUB_TOKEN="${HF_TOKEN}"
fi

normalize_limit_mm_per_prompt() {
  local raw="$1"
  local default_image="$2"
  local default_video="$3"
  local enforce_defaults="$4"
  python3 - "${raw}" "${default_image}" "${default_video}" "${enforce_defaults}" <<'PY'
import json
import sys

raw = sys.argv[1].strip()
default_image = int(sys.argv[2])
default_video = int(sys.argv[3])
enforce_defaults = sys.argv[4] == "1"
if not raw:
    print(json.dumps({"image": default_image, "video": default_video}, separators=(",", ":")))
    sys.exit(0)

# Preferred format: JSON object string
try:
    parsed = json.loads(raw)
    if isinstance(parsed, dict) and parsed:
        out = {}
        for k, v in parsed.items():
            out[str(k)] = int(v)
        if enforce_defaults:
            out.setdefault("image", default_image)
            out.setdefault("video", default_video)
        print(json.dumps(out, separators=(",", ":")))
        sys.exit(0)
except Exception:
    pass

# Backward-compatible format: image=1,video=0
out = {}
for part in raw.split(","):
    part = part.strip()
    if not part:
        continue
    if "=" not in part:
        continue
    key, value = part.split("=", 1)
    key = key.strip()
    value = value.strip()
    if not key:
        continue
    try:
        out[key] = int(value)
    except Exception:
        continue

if out:
    if enforce_defaults:
        out.setdefault("image", default_image)
        out.setdefault("video", default_video)
    print(json.dumps(out, separators=(",", ":")))
    sys.exit(0)

print(f"[inference-api] invalid LIMIT_MM_PER_PROMPT format: {raw}", file=sys.stderr)
print('[inference-api] expected JSON (e.g. {"image":1,"video":0}) or legacy image=1,video=0', file=sys.stderr)
sys.exit(2)
PY
}

LIMIT_MM_PER_PROMPT_JSON="$(
  normalize_limit_mm_per_prompt \
    "${LIMIT_MM_PER_PROMPT}" \
    "${MM_DEFAULT_IMAGE_LIMIT}" \
    "${MM_DEFAULT_VIDEO_LIMIT}" \
    "${ENFORCE_MM_DEFAULT_LIMITS}"
)"

normalize_base_url() {
  local raw="$1"
  raw="${raw%/}"
  if [[ "${raw}" == */v1 ]]; then
    printf '%s\n' "${raw}"
  else
    printf '%s/v1\n' "${raw}"
  fi
}

# If running on Vast, these are commonly present:
#   PUBLIC_IPADDR, VAST_TCP_PORT_<internal_port> (external port mapping)  (see Vast docs)
infer_vast_external_base_url() {
  if [[ -z "${PUBLIC_IPADDR:-}" ]]; then
    return 1
  fi
  local var="VAST_TCP_PORT_${PORT}"
  local ext="${!var:-}"
  if [[ -z "${ext}" ]]; then
    return 1
  fi
  printf 'http://%s:%s\n' "${PUBLIC_IPADDR}" "${ext}"
  return 0
}

infer_codex_server_url() {
  if [[ -n "${CODEX_SERVER_URL:-}" ]]; then
    normalize_base_url "${CODEX_SERVER_URL}"
    return
  fi
  if [[ -n "${PUBLIC_BASE_URL:-}" ]]; then
    normalize_base_url "${PUBLIC_BASE_URL}"
    return
  fi

  # Vast auto-detection (falls back if not on Vast)
  local vast_base=""
  if vast_base="$(infer_vast_external_base_url 2>/dev/null)"; then
    normalize_base_url "${vast_base}"
    return
  fi

  printf 'http://localhost:%s/v1\n' "${PORT}"
}

TOTAL_GPU_MIB="${TOTAL_GPU_MIB:-}"
if [[ -z "${TOTAL_GPU_MIB}" ]]; then
  if command -v nvidia-smi >/dev/null 2>&1; then
    TOTAL_GPU_MIB="$(nvidia-smi --query-gpu=memory.total --format=csv,noheader,nounits | head -n1 | tr -d ' ')"
  fi
fi
if [[ -z "${TOTAL_GPU_MIB}" ]]; then
  TOTAL_GPU_MIB=32768
fi

KV_BYTES_PER_TOKEN=131072
if [[ "${KV_CACHE_DTYPE}" != "fp8" && "${KV_CACHE_DTYPE}" != "fp8_e4m3" && "${KV_CACHE_DTYPE}" != "fp8_e5m2" ]]; then
  KV_BYTES_PER_TOKEN=262144
fi

if [[ -n "${MAX_MODEL_LEN:-}" ]]; then
  EFFECTIVE_MAX_MODEL_LEN="${MAX_MODEL_LEN}"
else
  EFFECTIVE_MAX_MODEL_LEN="$(
    TOTAL_GPU_MIB="${TOTAL_GPU_MIB}" \
    GPU_MEMORY_UTILIZATION="${GPU_MEMORY_UTILIZATION}" \
    MODEL_WEIGHTS_GIB="${MODEL_WEIGHTS_GIB}" \
    RUNTIME_OVERHEAD_GIB="${RUNTIME_OVERHEAD_GIB}" \
    KV_BYTES_PER_TOKEN="${KV_BYTES_PER_TOKEN}" \
    MAX_CONTEXT_HARD_CAP="${MAX_CONTEXT_HARD_CAP}" \
    MIN_CONTEXT_LEN="${MIN_CONTEXT_LEN}" \
    CONTEXT_RESERVE_TOKENS="${CONTEXT_RESERVE_TOKENS}" \
    CONTEXT_ROUNDING_STEP="${CONTEXT_ROUNDING_STEP}" \
    python3 - <<'PY'
import os

total_mib = float(os.environ["TOTAL_GPU_MIB"])
gpu_util = float(os.environ["GPU_MEMORY_UTILIZATION"])
weights_gib = float(os.environ["MODEL_WEIGHTS_GIB"])
overhead_gib = float(os.environ["RUNTIME_OVERHEAD_GIB"])
kv_bytes_per_token = int(os.environ["KV_BYTES_PER_TOKEN"])
hard_cap = int(os.environ["MAX_CONTEXT_HARD_CAP"])
min_ctx = int(os.environ["MIN_CONTEXT_LEN"])
reserve = int(os.environ["CONTEXT_RESERVE_TOKENS"])
step = int(os.environ["CONTEXT_ROUNDING_STEP"])
model_cap = 262144

usable_gib = (total_mib / 1024.0) * gpu_util
kv_budget_gib = usable_gib - weights_gib - overhead_gib
kv_budget_bytes = max(int(kv_budget_gib * (1024 ** 3)), 0)
raw_tokens = kv_budget_bytes // kv_bytes_per_token
safe_tokens = max(raw_tokens - reserve, min_ctx)
safe_tokens = min(safe_tokens, hard_cap, model_cap)
safe_tokens = max((safe_tokens // step) * step, min_ctx)
print(safe_tokens)
PY
  )"
fi

if [[ "${MAX_NUM_BATCHED_TOKENS}" == "0" ]]; then
  MAX_NUM_BATCHED_TOKENS="${EFFECTIVE_MAX_MODEL_LEN}"
fi
USER_MAX_NUM_BATCHED_TOKENS="${MAX_NUM_BATCHED_TOKENS}"

CODEX_URL="$(infer_codex_server_url)"

echo "[inference-api] model=${MODEL_ID}"
echo "[inference-api] served_model_name=${SERVED_MODEL_NAME}"
echo "[inference-api] total_gpu_mib=${TOTAL_GPU_MIB}"
echo "[inference-api] gpu_memory_utilization=${GPU_MEMORY_UTILIZATION}"
echo "[inference-api] kv_cache_dtype=${KV_CACHE_DTYPE}"
echo "[inference-api] limit_mm_per_prompt=${LIMIT_MM_PER_PROMPT_JSON}"
echo "[inference-api] mm_processor_cache_gb=${MM_PROCESSOR_CACHE_GB}"
echo "[inference-api] skip_mm_profiling=${SKIP_MM_PROFILING}"
echo "[inference-api] max_model_len=${EFFECTIVE_MAX_MODEL_LEN}"
echo "[inference-api] max_num_seqs=${MAX_NUM_SEQS}"
echo "[inference-api] swap_space_gb=${SWAP_SPACE_GB}"
echo ""
echo "[inference-api] Codex model switch field (server URL)"
echo "${CODEX_URL}"
echo ""

build_vllm_cmd() {
  local kv_dtype="$1"
  local -a cmd
  cmd=(
    vllm serve "${MODEL_ID}"
    --served-model-name "${SERVED_MODEL_NAME}"
    --host "${HOST}"
    --port "${PORT}"
    --gpu-memory-utilization "${GPU_MEMORY_UTILIZATION}"
    --max-model-len "${EFFECTIVE_MAX_MODEL_LEN}"
    --max-num-seqs "${MAX_NUM_SEQS}"
    --max-num-batched-tokens "${MAX_NUM_BATCHED_TOKENS}"
    --swap-space "${SWAP_SPACE_GB}"
    --tensor-parallel-size "${TENSOR_PARALLEL_SIZE}"
    --trust-remote-code
    --limit-mm-per-prompt "${LIMIT_MM_PER_PROMPT_JSON}"
    --mm-processor-cache-gb "${MM_PROCESSOR_CACHE_GB}"
    --kv-cache-dtype "${kv_dtype}"
  )

  if [[ "${SKIP_MM_PROFILING}" == "1" ]]; then
    cmd+=(--skip-mm-profiling)
  fi

  if [[ "${ENABLE_LOG_REQUESTS}" == "1" ]]; then
    cmd+=(--enable-log-requests)
  fi

  if [[ "${ENABLE_AUTO_TOOL_CHOICE}" == "1" ]]; then
    cmd+=(--enable-auto-tool-choice --tool-call-parser "${TOOL_CALL_PARSER}")
  fi

  printf '%q ' "${cmd[@]}"
}

set_runtime_lengths() {
  local max_len="$1"
  EFFECTIVE_MAX_MODEL_LEN="${max_len}"
  if [[ "${USER_MAX_NUM_BATCHED_TOKENS}" == "0" ]]; then
    MAX_NUM_BATCHED_TOKENS="${max_len}"
    return
  fi
  if (( USER_MAX_NUM_BATCHED_TOKENS > max_len )); then
    MAX_NUM_BATCHED_TOKENS="${max_len}"
  else
    MAX_NUM_BATCHED_TOKENS="${USER_MAX_NUM_BATCHED_TOKENS}"
  fi
}

build_context_candidates() {
  local start="$1"
  local -a out=("${start}")
  local step="${CONTEXT_FALLBACK_STEP}"
  local min_len="${MIN_STARTUP_MAX_MODEL_LEN}"

  if [[ "${ENABLE_STARTUP_CONTEXT_FALLBACK}" != "1" ]]; then
    printf '%s\n' "${out[@]}"
    return
  fi

  if (( step < 1024 )); then
    step=1024
  fi
  if (( min_len < 8192 )); then
    min_len=8192
  fi

  local next=$((start - step))
  while (( next >= min_len )); do
    out+=("${next}")
    next=$((next - step))
  done

  if (( start > min_len )); then
    local last_index=$(( ${#out[@]} - 1 ))
    if (( out[last_index] != min_len )); then
      out+=("${min_len}")
    fi
  fi

  printf '%s\n' "${out[@]}"
}

start_server() {
  local kv_dtype="$1"
  local cmd
  cmd="$(build_vllm_cmd "${kv_dtype}")"
  echo "[inference-api] starting vLLM with kv_cache_dtype=${kv_dtype}, max_model_len=${EFFECTIVE_MAX_MODEL_LEN}, max_num_batched_tokens=${MAX_NUM_BATCHED_TOKENS}"
  bash -lc "${cmd}"
}

run_with_fallbacks() {
  local -a kv_candidates=("${KV_CACHE_DTYPE}")
  if [[ "${ENABLE_KV_DTYPE_FALLBACK}" == "1" && "${KV_CACHE_DTYPE}" == "fp8" && "${KV_FALLBACK_DTYPE}" != "${KV_CACHE_DTYPE}" ]]; then
    kv_candidates+=("${KV_FALLBACK_DTYPE}")
  fi

  local -a context_candidates=()
  while IFS= read -r candidate; do
    context_candidates+=("${candidate}")
  done < <(build_context_candidates "${EFFECTIVE_MAX_MODEL_LEN}")

  local rc=1
  local kv=""
  local ctx=""
  for kv in "${kv_candidates[@]}"; do
    for ctx in "${context_candidates[@]}"; do
      set_runtime_lengths "${ctx}"
      set +e
      start_server "${kv}"
      rc=$?
      set -e
      if [[ ${rc} -eq 0 ]]; then
        return 0
      fi
      echo "[inference-api] vLLM exited with rc=${rc} for kv_cache_dtype=${kv}, max_model_len=${ctx}"
    done
  done
  return "${rc}"
}

if [[ "${VLLM_DAEMON}" == "1" ]]; then
  echo "[inference-api] starting vLLM supervisor in background"
  echo "[inference-api] log_file=${LOG_FILE}"
  bash -lc "nohup $(printf '%q' "${SELF_SCRIPT}") foreground > '${LOG_FILE}' 2>&1 & echo \$! > '${PID_FILE}'"
  echo "[inference-api] vLLM pid=$(cat "${PID_FILE}" 2>/dev/null || true)"
  exit 0
fi

run_with_fallbacks
