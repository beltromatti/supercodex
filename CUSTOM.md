# Codex Custom

This document describes the custom Codex version maintained in this repository.

Fork objective:

1. use a self-hosted model (`Qwen3-VL-32B-Instruct-AWQ`) transparently inside the standard Codex flow;
2. manage multiple ChatGPT accounts in a single client, with manual switching and automatic failover when an account hits a rate/usage limit.

The architecture principle is: minimal patches, integrated with existing Codex mechanisms, without breaking standard behavior for users who do not use custom features.

## Status of implemented modifications

This fork introduces two main modifications.

### Modification A: vLLM inference stack on 32 GB GPU + model wiring in Codex

Infrastructure part (`inference-api/`):

- container based on `vllm/vllm-openai:v0.11.0`;
- default model `QuantTrio/Qwen3-VL-32B-Instruct-AWQ`;
- OpenAI-compatible endpoint on `/v1/*`;
- automatic and safe `max_model_len` computation for 32 GB;
- FP8 KV cache as default with automatic fallback;
- daemon startup mode for provider environments (`/opt/start-vllm.sh --daemon`);
- runtime-only container for Vast Jupyter mode (Jupyter/SSH managed by Vast, not by the container);
- startup logs + pid file for operator control (`/workspace/vllm.log`, `/workspace/vllm.pid`).
- startup fallback chain for memory pressure: automatic retry with lower `max_model_len` (step-based) and optional KV dtype fallback.

Codex integration part:

- model preset added in `codex-rs/core/src/models_manager/model_presets.rs`:
  - slug: `qwen3-vl-32b-instruct-awq`
  - default effort: `none`
- fallback metadata in `codex-rs/core/src/models_manager/model_info.rs`:
  - reference context window: `57_344`
- in TUI model picker (`codex-rs/tui/src/chatwidget.rs`), when you select this model:
  - it shows a prompt for the vLLM server URL;
  - URL is validated/normalized to `http(s)://.../v1` format;
  - sends `Op::OverrideTurnContext` with `provider_base_url` set.
- dedicated provider persistence `qwen_vllm` in config via `AppEvent::PersistQwenVllmProvider`:
  - `codex-rs/tui/src/app_event.rs`
  - `codex-rs/tui/src/app.rs`
  - forced properties:
    - `requires_openai_auth = false`
    - `supports_websockets = false`
    - `wire_api = "responses"`

Protocol support:

- `provider_base_url` is present in `Op::OverrideTurnContext` (`codex-rs/protocol/src/protocol.rs`)
- applied in session config (`codex-rs/core/src/codex.rs`), with dynamic update of `provider.base_url`.

### Modification B: Multi-account ChatGPT + slash commands + auto-rotation on limits

Multi-account persistence:

- new account registry: `~/.codex/accounts.json`;
- implementation: `codex-rs/core/src/auth/account_registry.rs`;
- file separated from standard active account channel (`auth.json` / keyring / auto / ephemeral).

The existing active file is NOT replaced as a concept: it remains the single source of truth for the account in use, while `accounts.json` is only the saved account catalog.

Added TUI slash commands:

- `/accounts`: list saved accounts, with active account highlighting;
- `/addaccount`: launch local browser login and save account snapshot;
- `/removeaccount`: remove account from registry;
- `/swapaccount`: set a saved account as active.

Main files:

- `codex-rs/tui/src/slash_command.rs`
- `codex-rs/tui/src/chatwidget.rs`

Auto-rotation on limit error:

- in `run_sampling_request` (`codex-rs/core/src/codex.rs`), on `CodexErr::UsageLimitReached`:
  - updates rate-limit snapshot as upstream;
  - tries switch to next saved account not yet exhausted;
  - reloads auth manager;
  - resets websocket state to force handshake with new credentials;
  - retries the same request.
- websocket reset implemented in:
  - `codex-rs/core/src/client.rs` (`reset_websocket_state_for_auth_change`).

Fallback behavior:

- if there are no alternative accounts, or switch fails, it returns to the standard `UsageLimitReached` error flow.

## ChatGPT login: internal behavior

Login flow (personal account, not API key):

- local OAuth server in `codex-rs/login/src/server.rs`;
- default bind on `127.0.0.1:1455`;
- local callback `http://localhost:<port>/auth/callback`;
- automatic browser open (`webbrowser::open`).

After token exchange:

- tokens are persisted with `save_auth(...)`;
- storage backend depends on `AuthCredentialsStoreMode`:
  - `File`: `~/.codex/auth.json`
  - `Keyring`: OS keyring (with optional file fallback)
  - `Auto`: tries keyring, fallback to file
  - `Ephemeral`: runtime memory only

References:

- `codex-rs/core/src/auth.rs`
- `codex-rs/core/src/auth/storage.rs`
- `codex-rs/login/src/server.rs`

## Inference API: complete operational usage

For full technical details see `inference-api/README.md`.

Recommended workflow (Vast Jupyter mode):

1. launch Vast instance in `Jupyter-python notebook + SSH`;
2. expose only container port `8000/TCP` for vLLM API;
3. use On-start Script:
   - `bash -lc "/opt/start-vllm.sh --daemon"`
4. inspect logs from Jupyter terminal:
   - `tail -n 200 -f /workspace/vllm.log`
5. in Codex select model `Qwen3-VL-32B-Instruct-AWQ`;
6. paste the public endpoint with `/v1` suffix.

Alternative local workflow:

1. prepare `.env` from `.env.example` in `inference-api/`;
2. start with `docker compose up --build`;
3. read the Codex-ready URL printed in startup logs;
4. use that URL in the model switch prompt.

Key memory parameters (safe Vast boot profile):

- `GPU_MEMORY_UTILIZATION=0.90`
- `MAX_MODEL_LEN=24576`
- `MAX_NUM_SEQS=1`
- `SWAP_SPACE_GB=24`
- `KV_CACHE_DTYPE=auto`
- `ENABLE_KV_DTYPE_FALLBACK=0`
- `LIMIT_MM_PER_PROMPT={"image":1,"video":0}`

Then scale up gradually when stable.

If OOM:

1. lower `MAX_MODEL_LEN`;
2. then `GPU_MEMORY_UTILIZATION`;
3. then `LIMIT_MM_PER_PROMPT`;
4. then `MAX_NUM_SEQS`.

## Multi-account: complete operational usage

### Add account

Command:

```text
/addaccount
```

Effect:

- first saves (best effort) the current active account in registry;
- opens local browser login;
- when login completes, saves new account in `accounts.json`;
- reloads runtime auth.

### Show saved accounts

Command:

```text
/accounts
```

Effect:

- popup with account list and active account marker (`[ACTIVE]` + current).

### Change active account

Command:

```text
/swapaccount
```

Effect:

- copies selected entry auth into active auth backend;
- runs `auth_manager.reload()` to apply change immediately.

### Remove account

Command:

```text
/removeaccount
```

Effect:

- removes entry from `accounts.json` registry;
- does not forcibly invalidate current active session.

### Automatic failover on usage limit

When an account hits limit:

- warning log in event flow;
- automatic account switch attempt;
- retry current turn;
- cycle continues until available accounts are exhausted.

If all are exhausted:

- standard provider error is propagated as in upstream.

## Invariants to preserve (fork contract)

These points must not regress in future merges:

1. model `qwen3-vl-32b-instruct-awq` must remain selectable in model picker;
2. selecting that model must request server URL and set `provider_base_url`;
3. provider config `qwen_vllm` must be persisted with `requires_openai_auth=false` and `supports_websockets=false`;
4. commands `/accounts`, `/addaccount`, `/removeaccount`, `/swapaccount` must remain in slash menu;
5. `accounts.json` must remain separate from `auth.json`/keyring;
6. on `UsageLimitReached` in ChatGPT auth mode, account rotation + retry must remain active;
7. websocket reset after account swap must remain in place.

## Update from upstream main: merge and conflict policy

This section defines official fork policy.

### General principle

- upstream must be integrated regularly;
- on custom feature conflict points, custom behavior has priority;
- prefer minimal and localized merges, without rewriting whole blocks unless necessary.

### High-sensitivity files (likely conflicts)

Inference:

- `inference-api/Dockerfile`
- `inference-api/start-vllm.sh`
- `inference-api/docker-compose.yaml`
- `inference-api/.env.example`
- `inference-api/README.md`

Model wiring:

- `codex-rs/core/src/models_manager/model_presets.rs`
- `codex-rs/core/src/models_manager/model_info.rs`
- `codex-rs/protocol/src/protocol.rs`
- `codex-rs/core/src/codex.rs`
- `codex-rs/tui/src/chatwidget.rs`
- `codex-rs/tui/src/app_event.rs`
- `codex-rs/tui/src/app.rs`

Multi-account:

- `codex-rs/core/src/auth.rs`
- `codex-rs/core/src/auth/account_registry.rs`
- `codex-rs/core/src/client.rs`
- `codex-rs/core/src/codex.rs`
- `codex-rs/tui/src/slash_command.rs`
- `codex-rs/tui/src/chatwidget.rs`

Sensitive tests:

- `codex-rs/core/tests/suite/list_models.rs`
- `codex-rs/core/tests/suite/model_switching.rs`
- `codex-rs/core/tests/suite/model_overrides.rs`
- `codex-rs/core/tests/suite/grep_files.rs`

### Recommended merge procedure

Example to run from repo root:

```bash
git remote add upstream https://github.com/openai/codex.git
git fetch upstream
git checkout main
git pull --ff-only origin main
git merge upstream/main
```

If conflicts happen:

1. resolve non-custom files first following upstream;
2. on high-sensitivity files, preserve custom invariants;
3. avoid unnecessary refactors during conflict resolution;
4. confirm with targeted tests + full build.

When a conflict accidentally removes custom behavior, reapply minimal surgical patch.

### Practical conflict-resolution strategy

For files where custom logic has priority, you can start from local version and then re-integrate useful/non-conflicting upstream updates:

```bash
git checkout --ours <file>
# manually reintroduce only useful/non-conflicting upstream parts
```

For files where upstream introduces critical security/bug fixes:

```bash
git checkout --theirs <file>
# then reapply custom patches required by invariants
```

Preferred decision:

- `ours` for custom model wiring, account slash commands, and automatic rotation;
- `theirs` only if upstream changes core interfaces incompatibly and you must realign first.

### Mandatory post-merge checklist

From `codex-rs/`:

```bash
cargo fmt --all
cargo build -p codex-cli --bin codex
cargo test -p codex-core -p codex-tui
```

Minimum manual functional verification:

1. `/model` shows `Qwen3-VL-32B-Instruct-AWQ`;
2. model selection opens server URL prompt;
3. `/addaccount`, `/accounts`, `/swapaccount`, `/removeaccount` are available and working;
4. account switch effectively updates active account;
5. on simulated usage-limit, switch attempt and retry are visible.

## Future maintenance policy

Operational rules:

1. every custom change must remain tracked in this file;
2. new custom features must not break standard OpenAI/API key paths;
3. avoid massive divergence: prefer hooks and extensions on existing flows;
4. every upstream merge must close only with green core+tui suite;
5. if upstream protocol/event contract changes, update together:
   - core
   - tui
   - any integration tests
   to avoid functional drift.

## Quick reference files

- Inference stack: `inference-api/README.md`
- Account registry: `codex-rs/core/src/auth/account_registry.rs`
- Account slash commands: `codex-rs/tui/src/slash_command.rs`
- Account UI flow + vLLM prompt: `codex-rs/tui/src/chatwidget.rs`
- Qwen provider persistence: `codex-rs/tui/src/app.rs`
- Auto-rotation on limits: `codex-rs/core/src/codex.rs`
- Websocket reset after swap: `codex-rs/core/src/client.rs`
