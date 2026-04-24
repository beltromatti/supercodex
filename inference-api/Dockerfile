FROM vllm/vllm-openai:v0.11.0

USER root

# System tooling for interactive sessions.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      curl ca-certificates \
      git git-lfs \
      tmux htop jq \
    && rm -rf /var/lib/apt/lists/*

COPY start-vllm.sh /opt/start-vllm.sh
RUN chmod +x /opt/start-vllm.sh

ENV HF_HOME=/data/huggingface \
    VLLM_CONFIG_ROOT=/data/vllm \
    PYTHONUNBUFFERED=1

WORKDIR /workspace

EXPOSE 8000

HEALTHCHECK --interval=30s --timeout=10s --start-period=60s --retries=5 \
  CMD curl -fsS http://127.0.0.1:${PORT:-8000}/health || exit 1

# ENTRYPOINT is used for raw Docker runs.
# On Vast Jupyter mode you can start vLLM via On-start Script with:
# /opt/start-vllm.sh --daemon
ENTRYPOINT ["/opt/start-vllm.sh"]
