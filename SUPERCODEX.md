# Super Codex — Technical Notes

This document is the authoritative technical reference for the Super
Codex fork: what it is based on, what it changes relative to upstream
[`openai/codex`](https://github.com/openai/codex), how those changes are
structured in the tree, and how releases are produced.

It is intended for contributors and operators who need to reason about
the fork end-to-end. For the high-level vision and disclaimer, see
[`README.md`](README.md). For contribution policy, see
[`docs/contributing.md`](docs/contributing.md).

---

## 1. Relationship to OpenAI Codex

- **Upstream**: [openai/codex](https://github.com/openai/codex),
  Apache-2.0.
- **Base tag** for the current Super Codex line: `rust-v0.124.0`
  (upstream). Previous base was `rust-v0.121.0`; the 0.121 → 0.124
  rebase integrated 325 upstream commits and touched 49 files that
  Super Codex also owns (Op enum layout, auth manager lock type
  semaphore → mutex migration, session/* refactor splitting the
  old codex.rs into turn.rs + handlers.rs + session.rs, TUI app/
  submodule split, new collaboration-modes / permission-profile /
  model-catalog / stream-controller scaffolds). Every fork
  feature was re-ported on top of the new layout with
  `cargo check --workspace --all-targets` clean and all account-
  registry regression tests green.
- **Super Codex tag**: `super-vX.Y.Z`, with `X.Y.Z` **strictly 1:1**
  with the upstream stable (`rust-vX.Y.Z`) the fork is currently
  rebased onto. Fork-only fixes (multi-account tweaks, branding
  patches, workflow adjustments, ...) accumulate on `main` between
  merge windows and ship **with the next upstream stable bump** —
  there are no intermediate `super-v0.121.1`, `super-v0.121.2`
  patch releases on top of the same base. This keeps Super Codex
  versioning permanently aligned with upstream and avoids a drifting
  patch-number tail.
- **Distribution**: npm package
  [`@beltromatti/supercodex`](https://www.npmjs.com/package/@beltromatti/supercodex)
  and GitHub Releases on
  [`beltromatti/supercodex`](https://github.com/beltromatti/supercodex).

The fork is maintained as a single `main` branch. Every commit in
`main` that is not present in the upstream tag it was rebased onto is
a Super Codex modification. There is no long-lived patch branch; the
rebase-on-merge-window model (section 6) keeps the divergence small
and visible in `git log`.

---

## 2. Feature areas owned by the fork

### 2.1 Multi-account ChatGPT management

A persistent registry of ChatGPT logins, with in-session switching and
automatic rotation when the active account hits a usage limit.

**New module** — [`codex-rs/login/src/auth/account_registry.rs`](codex-rs/login/src/auth/account_registry.rs):

- Serializes saved accounts to `$CODEX_HOME/accounts.json` with a
  versioned schema (`SavedChatgptAccountsFile { version, accounts }`).
- Derives a stable id for each account from the tokens in
  [`AuthDotJson`](codex-rs/login/src/auth/storage.rs) (account_id →
  user_id → normalised email, in that order). This id is what
  `/accounts` and the auto-rotation path use to key on.
- Public API surface:
  - `list_saved_chatgpt_accounts`
  - `upsert_saved_chatgpt_account`, `upsert_active_chatgpt_account`
  - `remove_saved_chatgpt_account`
  - `current_saved_chatgpt_account_id`
  - `switch_active_chatgpt_account`
  - `rotate_to_next_saved_chatgpt_account`
  - `account_registry_id_for_auth`
- Re-exported through
  [`codex-rs/login/src/auth/mod.rs`](codex-rs/login/src/auth/mod.rs)
  and [`codex-rs/login/src/lib.rs`](codex-rs/login/src/lib.rs) so the
  TUI and core can reach it via `codex_login::…`.
- `AuthManager::resolved_mode` was promoted to `pub(super)` so the
  registry can gate on ChatGPT-vs-ApiKey auth.

**Auto-rotation on `UsageLimitReached`** —
[`codex-rs/core/src/session/turn.rs::run_sampling_request`](codex-rs/core/src/session/turn.rs):

- Tracks the set of exhausted account ids for the current turn in a
  per-loop `HashSet<String>`.
- On `CodexErr::UsageLimitReached`, iff the active auth is a ChatGPT
  auth (`CodexAuth::is_chatgpt_auth`), the loop:
  1. Emits a `Warning` event to the TUI about the hit account.
  2. Calls `AuthManager::rotate_to_next_saved_chatgpt_account` (the
     lock-coordinated method — see "Lock-coordinated switch" below),
     which picks the next saved account not already exhausted,
     rewrites `$CODEX_HOME/auth.json`, and refreshes the cached auth
     under the refresh lock.
  3. Calls `client_session.reset_websocket_session()` (the 0.124
     successor to the removed `reset_websocket_state_for_auth_change`)
     so the retry reconnects with fresh auth headers on WebSocket
     transport.
  4. Resets `retries = 0` and `continue`s the loop — the current
     turn is re-sent under the new account with no session restart.
- If no alternative saved account is eligible, the original
  `UsageLimitReached` error is surfaced to the user unchanged.

**Slash commands** —
[`codex-rs/tui/src/slash_command.rs`](codex-rs/tui/src/slash_command.rs)
adds three variants: `Accounts`, `AddAccount`, `RemoveAccount`.
Dispatch is wired in
[`codex-rs/tui/src/chatwidget/slash_dispatch.rs`](codex-rs/tui/src/chatwidget/slash_dispatch.rs).
Implementation helpers (`open_accounts_list_popup`,
`open_remove_account_popup`, `start_add_account_login`,
`load_saved_accounts_with_current`) live in
[`codex-rs/tui/src/chatwidget.rs`](codex-rs/tui/src/chatwidget.rs).

`/accounts` is the single entry point: it renders the saved accounts
as an actionable selection view. ↑/↓ moves the highlight, Enter
switches the active account to the selected one. The currently
active account is marked `[ACTIVE]`, appears as `is_current`, and
is disabled so it cannot be re-selected.

`/addaccount` spawns a local OAuth flow via
`codex_login::run_login_server` and, on success, persists the newly
active credentials into the registry and reloads the in-memory auth
so subsequent requests use the new tokens without a session restart.

`/removeaccount` deletes the chosen registry entry. If the removed
entry was the **currently active** account the App layer
automatically rotates to the first remaining saved account (same
lock-coordinated path as `/accounts`); one combined info message
reports both the remove and the rotation. If no other saved
accounts remain the user is pointed at `/addaccount`.

**Routing via the shared `Arc<AuthManager>`** — three TUI-internal
events deliver the operations directly to the `AuthManager` held by
the embedded app-server, without going through the app-server's
typed RPC surface:

- [`AppEvent::SwitchChatgptAccountRequested { account_id }`](codex-rs/tui/src/app_event.rs) — emitted by the `/accounts` Enter action.
- `AppEvent::ReloadAuthRequested` — emitted after `/addaccount`.
- `AppEvent::RemoveChatgptAccountRequested { account_id, label }` — emitted by `/removeaccount`.

Each handler in [`codex-rs/tui/src/app.rs`](codex-rs/tui/src/app.rs)
obtains the `Arc<AuthManager>` via
`AppServerSession::auth_manager()` (returns `Some(..)` for the
embedded runtime, `None` for a remote app-server — multi-account
management is a local-CLI feature and surfaces a clean
"not available in remote mode" message in that case). It then
invokes:

- `AuthManager::switch_to_saved_chatgpt_account` and
- `AuthManager::rotate_to_next_saved_chatgpt_account` in
  [`codex-rs/login/src/auth/manager.rs`](codex-rs/login/src/auth/manager.rs):
  both acquire the same `refresh_lock` that
  `AuthManager::refresh_token` takes, then delegate to the free
  functions in
  [`account_registry`](codex-rs/login/src/auth/account_registry.rs)
  that rewrite `$CODEX_HOME/auth.json`, and finally call
  `reload()` so the cached auth matches disk before releasing the
  lock.

The `Arc<AuthManager>` is plumbed bottom-up so the TUI and the
embedded core hold **the same instance**, guaranteeing that a
switch reloads core's cache and not only the TUI's:

- [`codex-rs/app-server/src/in_process.rs`](codex-rs/app-server/src/in_process.rs)
  — `InProcessClientHandle::auth_manager()` retains the Arc built
  at runtime startup.
- [`codex-rs/app-server-client/src/lib.rs`](codex-rs/app-server-client/src/lib.rs)
  — `InProcessAppServerClient::auth_manager()` forwards it; the
  `AppServerClient` enum wraps it as
  `Option<Arc<AuthManager>>` so remote mode degrades cleanly.
- [`codex-rs/tui/src/app_server_session.rs`](codex-rs/tui/src/app_server_session.rs)
  — `AppServerSession::auth_manager()` + `read_status_account_state()`
  let the TUI refresh the status card after a switch by re-reading
  from the same source of truth the app-server uses.

**Status card refresh** — the `/status` Account line, the session
info cell, and the connectors gating all read from
`ChatWidget::status_account_display` / `plan_type` /
`has_chatgpt_account`. Those fields are populated once at
`AppServerBootstrap` and would otherwise stay pinned to the
boot-time snapshot, so a user who switched mid-session would see
"Account: old@..." in `/status` even though the next HTTP request
already rides the new bearer. `App::refresh_status_account_from_server`
in [`codex-rs/tui/src/app/account_management.rs`](codex-rs/tui/src/app/account_management.rs)
re-runs the `read_account` RPC (same one `bootstrap` uses), maps
the response through `AppServerSession::read_status_account_state`,
and pushes the result into the chat widget via
`ChatWidget::update_account_state`. It is called from:

- `handle_switch_chatgpt_account` after a successful `/accounts`
  switch;
- `handle_reload_auth` after `/addaccount`;
- `handle_remove_chatgpt_account` after the
  remove-then-auto-rotate path swaps in the next saved account;
- indirectly for the in-core auto-rotation: `ChatWidget::on_warning`
  in [`codex-rs/tui/src/chatwidget.rs`](codex-rs/tui/src/chatwidget.rs)
  detects the `"Usage limit hit on this account; switched to"`
  warning the core's `run_sampling_request` emits after an
  auto-rotate and schedules an `AppEvent::ReloadAuthRequested`,
  which reaches the same handler chain.

Why **not** a new RPC: the 0.121 app-server dispatch (`ClientRequest`
enum in
[`codex-rs/app-server-protocol/`](codex-rs/app-server-protocol/))
routes TUI→core operations through typed methods like
`thread_compact_start` or `reload_user_config`, not a generic
`Op` passthrough. An earlier Super Codex attempt added
`Op::SwitchChatgptAccount` / `Op::ReloadAuth` variants and emitted
them via `AppEvent::CodexOp`; the TUI's `submit_active_thread_op`
then mapped them to `AppCommandView::Other`, no dispatch arm matched,
and the user saw `■ Not available in TUI yet for thread <id>` with
the switch silently aborted. Sharing the Arc directly avoids
adding boilerplate across the protocol / server / client / TUI
layers while keeping the refresh-lock coordination intact.

Why the lock is necessary: upstream's `refresh_token` captures an
`expected_account_id` when it begins, and if it later finds that
`auth.json` on disk carries a different `account_id`, it refuses
the refresh with a permanent `ACCOUNT_MISMATCH` error ("Your
access token could not be refreshed because you have since logged
out or signed in to another account. Please sign in again."). If a
token refresh happens to be in flight when the user switches
accounts, that guard fires even though the switch is intentional
and the user does not need to re-authenticate. Serialising the
switch on the same `refresh_lock` means an in-flight refresh
completes under the old account before the swap takes effect, and
any refresh that starts after the swap sees the new account id and
matches cleanly. Applies to both manual `/accounts` switches and
the automatic usage-limit rotation path in
`run_sampling_request`.

**Registry-snapshot freshness** — the account registry stores a
full `AuthDotJson` for every saved account, which is the blob
written back to `$CODEX_HOME/auth.json` on switch. Without extra
care, that blob is merely a photograph taken when the account was
first added: the tokens inside it never track the refreshes that
happen while the account is active, so a later switch back to the
same account could restore an already-rotated `refresh_token` and
put the user into a "Please sign in again" loop that had nothing
to do with their actual credentials.

`AuthManager::mirror_active_auth_into_registry` is the hook that
fixes this. It grabs the currently-cached auth, and if it is a
ChatGPT auth, refreshes the corresponding registry entry with the
fresh `AuthDotJson` (scoped to the auth's stable registry id).
The hook is called from two places:

- At the tail of every successful token refresh —
  `refresh_and_persist_chatgpt_token` and `refresh_external_auth`
  — so the registry entry for the active account advances in
  lockstep with `auth.json`.
- At the top of `switch_to_saved_chatgpt_account` and
  `rotate_to_next_saved_chatgpt_account`, under the `refresh_lock`,
  immediately before `auth.json` is overwritten with the target
  account — so any refresh that landed between the previous switch
  and this one is still captured even if the continuous-mirror
  hook missed it.

Crucially, the hook is **update-only**: it calls
`account_registry::update_saved_chatgpt_account_if_exists`, which
rewrites an existing row in place but never inserts a new one.
An earlier implementation used the insert-or-update
`upsert_saved_chatgpt_account` and caused a regression where
`/removeaccount` on the active account silently re-added the
entry during the post-remove auto-rotate: the mirror fires before
the switch overwrites `auth.json`, so the in-memory cache still
carries the removed account's auth, and the insert path re-
created the entry — the user saw "Removed X. Switched to Y." but
X stayed in `/accounts` and subsequent `/removeaccount` cycles
just rotated through the three entries forever. The registry
unit test `update_if_exists_does_not_reinsert_removed_account`
locks the new semantic in.

What the pair of refresh-lock + registry-mirror does NOT cover:
if the saved `refresh_token` of a given account has been genuinely
invalidated server-side — expired, explicitly revoked on
chatgpt.com, or rotated by upstream — no amount of client-side
coordination can make the refresh succeed. The client cannot see
the refresh token's TTL (it is an opaque token, not a JWT); it
learns about the invalidation only from the three distinct server
errors upstream's `AuthManager` translates into messages
(`REFRESH_TOKEN_EXPIRED_MESSAGE`, `REFRESH_TOKEN_REUSED_MESSAGE`,
`REFRESH_TOKEN_INVALIDATED_MESSAGE`). When any of those surfaces,
the user does need to `/addaccount` to re-login that specific
account.

### 2.2 Self-hosted Qwen3-VL AWQ provider

Super Codex ships first-class support for running the CLI against a
self-hosted [vLLM](https://github.com/vllm-project/vllm) server that
serves `QuantTrio/Qwen3-VL-32B-Instruct-AWQ`.

**Catalog entry** — [`codex-rs/models-manager/models.json`](codex-rs/models-manager/models.json):

- Adds a `qwen3-vl-32b-instruct-awq` slug with `visibility: list`,
  `priority: 99`, text + image input modalities, 57 344-token context
  window, and a single `none` reasoning effort level.
- The standard upstream model catalog pipeline
  (`codex_models_manager::ModelsManager::build_available_models`)
  picks it up and exposes it in `/model`.

**ModelInfo fallback** —
[`codex-rs/models-manager/src/model_info.rs`](codex-rs/models-manager/src/model_info.rs):

- `model_info_from_slug` special-cases the Qwen slug and returns a
  hand-built descriptor (display name, description, 57 344-token
  context) instead of the generic 272 000-token fallback.

**`provider_base_url` as a first-class override**:

- New field added to
  [`Op::OverrideTurnContext`](codex-rs/protocol/src/protocol.rs),
  `SessionSettingsUpdate` in
  [`codex-rs/core/src/session/session.rs`](codex-rs/core/src/session/session.rs),
  and the TUI-side `AppCommand::override_turn_context` helper in
  [`codex-rs/tui/src/app_command.rs`](codex-rs/tui/src/app_command.rs).
- Applied in `SessionConfiguration::apply` so mid-session the provider
  base URL can be changed without restarting the session.
- All call sites of `Op::OverrideTurnContext` (~60 across workspace,
  mostly tests) updated to pass `provider_base_url: None` by default.
- The `AppCommandView::OverrideTurnContext` projection ignores the
  field with `provider_base_url: _,` — it is operational state, not a
  view concern.

**Qwen picker UX** — [`codex-rs/tui/src/chatwidget.rs`](codex-rs/tui/src/chatwidget.rs):

- `apply_model_and_effort` intercepts the Qwen slug and opens a
  `CustomPromptView` asking for the vLLM server URL instead of
  switching straight away.
- `open_vllm_server_prompt_for_model` normalises the URL
  (auto-appends `/v1`), fires
  `Op::OverrideTurnContext { provider_base_url: Some(url), … }` for
  the live session, and triggers `AppEvent::PersistQwenVllmProvider`
  to write a `[model_providers.qwen_vllm]` block into
  `config.toml` so subsequent launches don't re-prompt.

**Auto-reset on model change** —
[`codex-rs/tui/src/app.rs`](codex-rs/tui/src/app.rs):

- Helper `should_reset_provider_to_openai` returns `true` when the
  persisted provider is `qwen_vllm` and the newly selected model is
  **not** the Qwen slug.
- In the `PersistModelSelection` handler, when that condition holds,
  the in-memory config (`model_provider_id`, `model_provider`) is
  flipped back to the `openai` entry from `model_providers`, the
  status line refreshed, and the TOML edit also bumps the
  `model_provider` key on disk to `"openai"`.
- Unit tests cover the three branches of the helper.

### 2.3 `inference-api/` — self-hosted vLLM stack

A self-contained Docker + shell scaffold to stand up the vLLM server
that serves Qwen3-VL AWQ to Super Codex.

- [`inference-api/Dockerfile`](inference-api/Dockerfile) — CUDA + vLLM
  image, Python deps, entrypoint.
- [`inference-api/docker-compose.yaml`](inference-api/docker-compose.yaml)
  — single-node Compose recipe suitable for a RunPod / Vast.ai
  container with a single 32 GB NVIDIA GPU.
- [`inference-api/start-vllm.sh`](inference-api/start-vllm.sh) —
  boots the OpenAI-compatible `/v1` server with retries and
  memory-tuned `--max-model-len` / `--gpu-memory-utilization` /
  `--kv-cache-dtype fp8` defaults that land on 49 152 tokens of usable
  context at 0.95 utilisation. Runs as a daemon inside Vast Jupyter
  runtime mode.
- [`inference-api/README.md`](inference-api/README.md) — operator
  runbook (start/stop, troubleshooting, env vars).
- [`inference-api/.env.example`](inference-api/.env.example) — tuned
  defaults for a 32 GB GPU.

This directory is entirely additive and does not touch the upstream
codebase.

### 2.4 Branding and presentation

- **Binary name** — [`codex-rs/cli/Cargo.toml`](codex-rs/cli/Cargo.toml)
  renames `[[bin]].name` from `codex` to `supercodex`. Clap is also
  told to use `name = "supercodex"` and `bin_name = "supercodex"` in
  [`codex-rs/cli/src/main.rs`](codex-rs/cli/src/main.rs) so
  `--version`, `--help` and generated shell completions all show the
  branded name.
- **npm package** — [`codex-cli/package.json`](codex-cli/package.json)
  publishes as `@beltromatti/supercodex` with bin entry `supercodex`.
  The previous unscoped name was rejected by the npm registry's
  "too similar to an existing name" filter against `supercodex`; the
  scoped name is what npm itself suggested.
- **Startup splash** — a dedicated `SplashHistoryCell` and
  `new_super_codex_splash` function in
  [`codex-rs/tui/src/history_cell.rs`](codex-rs/tui/src/history_cell.rs)
  build a 6-line ANSI Shadow ASCII "SUPER CODEX" banner (cyan) plus
  a dim subtitle. `ChatWidget::new` in
  [`codex-rs/tui/src/chatwidget.rs`](codex-rs/tui/src/chatwidget.rs)
  paints the splash as the **first** history entry before the widget
  is returned, so it anchors the very top of every `supercodex` run
  regardless of how long the session configuration takes. A one-shot
  flag `super_codex_splash_shown` on `ChatWidget` (not upstream's
  `show_welcome_banner`, which is `is_first_run` and only true the
  very first time the TUI runs in a directory) guards the paint so
  in-process `/new` / `/resume` / `/fork` sessions do not repeat
  the banner.

  Upstream drew a dim-italic "loading..." placeholder
  `SessionHeaderHistoryCell` in `ChatWidget::new` (via
  `placeholder_session_header_cell`, wired into the bottom pane's
  `active_cell` slot) to hint at the session while the app-server
  was configuring. That placeholder's bottom-pane render was
  escaping into the terminal scrollback above the splash on
  non-alt-screen setups, producing a visually duplicate "loading"
  box ahead of the branding. `placeholder_session_header_cell` is
  removed and `active_cell` starts as `None`; the real session
  header arrives as a plain history cell when `SessionConfigured`
  fires, so the user sees exactly **splash → session header**
  in scrollback, no leftover ghost box.

  **Why the current art was picked.** Two wrap paths would normally
  try to re-flow a too-wide banner onto multiple rows on a narrow
  terminal:

  1. The viewport render — `impl Renderable for Box<dyn HistoryCell>`
     wraps with `Paragraph::wrap(Wrap { trim: false })`.
  2. The scrollback-commit render —
     `insert_history::insert_history_lines_with_mode` pre-wraps
     every non-URL line via `adaptive_wrap_line(width)` before
     writing to terminal output.

  Because the splash is committed to scrollback at
  `ChatWidget::new`, path 2 decides how the banner looks on boot,
  and a row wider than the viewport would stay folded in the
  terminal's scrollback forever (emulators freeze scrollback
  once committed, even on resize).

  `adaptive_wrap_line` cannot be opted out of cleanly without
  destabilising logic it shares with every other cell (URL-line
  rendering, mixed-token handling). The fork therefore uses a
  compact 5-row "Standard" figlet rendering of "SUPER CODEX" whose
  widest row is 63 cells — well below the 80-column floor of any
  reasonable terminal. That lets a single banner look the same
  everywhere, without branching on width, and guarantees
  `adaptive_wrap_line` never has anything to fold.

  Safety net for path 1: the trait method
  `HistoryCell::wraps_at_viewport_edge()` defaults to `true`
  upstream and is overridden to `false` on `SplashHistoryCell`;
  the render path in `history_cell.rs` then builds the
  `Paragraph` without `.wrap()` when that flag is off, so a
  future-proofing render through the active viewport still
  degrades to horizontal clipping instead of re-flowing.
- **Session header** — the same file renames the title line inside
  the `SessionHeaderHistoryCell` box from "OpenAI Codex" to
  "Super Codex". The `(vX.Y.Z)` suffix is unchanged.
- **`/status` card** — the card header in
  [`codex-rs/tui/src/status/card.rs`](codex-rs/tui/src/status/card.rs)
  renders its own `>_ OpenAI Codex (vX)` line independent of the
  session header; it is rebranded to `>_ Super Codex (vX)` to keep
  the two surfaces coherent.
- **Update checker** —
  [`codex-rs/tui/src/updates.rs`](codex-rs/tui/src/updates.rs) points
  the "newer version available" poller at
  `api.github.com/repos/beltromatti/supercodex/releases/latest`
  instead of `openai/codex`, so the notice the TUI shows at startup
  reflects Super Codex releases, not upstream ones.
- **README + banner** — [`README.md`](README.md) is brand-new (vision,
  "what's different", reference to upstream docs, liability
  disclaimer). [`.github/super-codex-banner.svg`](.github/super-codex-banner.svg)
  is the gradient title used at the top of the README.

---

## 3. Surface removed relative to upstream

Super Codex drops infrastructure that only existed to drive OpenAI
Codex's own CI, release, and contributor-bot pipelines. **No end-user
feature is removed.**

- `.github/workflows/` — all 22 upstream workflow YAMLs, replaced by a
  single [`.github/workflows/release.yml`](.github/workflows/release.yml).
- `.github/actions/` — the six custom Actions (`{macos,linux,windows}-code-sign`,
  `prepare-bazel-ci`, `setup-bazel-ci`, `setup-rusty-v8-musl`) were
  referenced only by the deleted workflows.
- `.github/scripts/` — twelve helper scripts for Bazel / rusty-v8 /
  argument-comment-lint, all orphaned by the workflow removals.
- `.github/codex/` — config for OpenAI's Auto-PR-Reviewer bot.
- `.github/dotslash-*.json`, `.github/blob-size-allowlist.txt`,
  `.github/codex-cli-splash.png` — upstream-only assets.
- `.github/ISSUE_TEMPLATE/` — six product-specific templates (Codex
  App, IDE Extension, CLI, etc.), replaced by a single
  [`bug-report.yml`](.github/ISSUE_TEMPLATE/bug-report.yml) form.
- `.github/pull_request_template.md` — rewritten to route PRs through
  [`docs/contributing.md`](docs/contributing.md) instead of upstream's.
- `docs/CONTRIBUTING.md` — duplicate of `docs/contributing.md`;
  consolidated into the latter, which is now Super Codex's own
  contributing guide.
- `.github/dependabot.yaml` — trimmed from six ecosystems to only
  `github-actions` (the only one whose deps Super Codex actually owns;
  Rust / Docker / devcontainer bumps arrive through the upstream merge
  window and Dependabot PRs on those would only create conflicts).
- `CUSTOM.md` — earlier fork README draft, folded into `README.md`.

---

## 4. Release pipeline

### 4.1 Trigger

The workflow listens on `push: tags: ["super-v*.*.*"]`. Any other
push — to `main`, to feature branches, to upstream-style `rust-v*`
tags, to Dependabot PRs — is a no-op.

### 4.2 Version injection

The tag `super-vX.Y.Z` is stripped of its prefix in the `prepare`
job (regex `super-v`) and the resulting semver is piped as the
workflow output `version`. The version is injected in two places
before the build runs:

- In `codex-rs/Cargo.toml`, into the `[workspace.package] version`
  key. All workspace crates inherit it via `version.workspace = true`.
- In `codex-cli/package.json`, into the `version` field before
  `npm publish`.

This replaces the `0.0.0-dev` placeholder committed to `main`, so the
binary's `supercodex --version` and the npm tarball both carry the
real tag version.

### 4.3 Build matrix

Three targets, one job each, on standard GitHub-hosted runners so the
workflow stays free of charge for public repos:

| Target | Runner | Binary |
|---|---|---|
| `aarch64-apple-darwin` | `macos-latest` | `supercodex` |
| `x86_64-unknown-linux-gnu` | `ubuntu-24.04` | `supercodex` |
| `x86_64-pc-windows-msvc` | `windows-latest` | `supercodex.exe` |

Each job:

1. Installs `libcap-dev` + `pkg-config` on Linux (required by
   `codex-linux-sandbox`'s vendored bubblewrap build).
2. Exports `CARGO_NET_GIT_FETCH_WITH_CLI=true` and
   `CARGO_NET_RETRY=10` — libgit2 (cargo's default) chokes on
   Chromium's `googlesource.com` endpoint that `libwebrtc` pulls as
   a submodule on macOS builds of `codex-realtime-webrtc`.
3. Runs `cargo build --release --target <triple> -p codex-cli` with
   a three-attempt retry loop to absorb remaining transient network
   flakiness.
4. Stages artifacts: both the versioned raw binary
   (`supercodex-<version>-<triple>[.exe]`) **and** a tar.gz (or zip on
   Windows) archive containing the same binary.

### 4.4 Publishing

The `release` job, after the three builds succeed:

1. Downloads every artifact.
2. Creates a GitHub Release at the triggering tag with the raw
   versioned binaries **and** archives attached — this happens
   **before** `npm publish` so the binaries are reachable from the
   moment the npm tarball lands (the postinstall script hits the
   GitHub Release URL — see 4.5).
3. Stamps `codex-cli/package.json` with the release version.
4. Runs `npm publish --access public` on `codex-cli/`, authenticated
   via the `NPM_TOKEN` repo secret (an npm Automation token that
   bypasses 2FA).

### 4.5 npm package layout — postinstall binary fetch

[`codex-cli/package.json`](codex-cli/package.json) ships without any
vendored binaries: `files` is `["bin", "scripts"]`, the unpacked size
is ~50 kB, and a `postinstall: node scripts/install.mjs` runs on the
user's machine at install time.

[`codex-cli/scripts/install.mjs`](codex-cli/scripts/install.mjs):

- Detects platform + arch, maps to one of the three supported
  triples (rejects with a clear error otherwise).
- Reads the package's own `version` out of `package.json`.
- Downloads `supercodex-<version>-<triple>[.exe]` from
  `https://github.com/beltromatti/supercodex/releases/download/super-v<version>/`
  into `vendor/<triple>/codex/supercodex[.exe]` inside the package.
- Follows redirects, chmods `0o755` on Unix, and exits non-zero on
  failure with a printable manual-download URL.

[`codex-cli/bin/codex.js`](codex-cli/bin/codex.js) — the npm bin entry
points at this file — detects the host triple at run time and spawns
`vendor/<triple>/codex/supercodex[.exe]` with the user's argv,
forwarding `SIGINT`/`SIGTERM`/`SIGHUP` and mirroring the child's exit
code or signal.

The result is:

- Small npm tarball (no 200 MB payload rejections).
- Single `npm install -g @beltromatti/supercodex` lands a working
  `supercodex` command.
- Offline reinstalls still work because the binary lives inside the
  package (at `vendor/...`) after the first install.

---

## 5. Local development

All Rust work happens inside `codex-rs/`.

```bash
# Compile everything the release runs through
cd codex-rs
cargo check --workspace --all-targets

# Build the release binary
cargo build --release -p codex-cli
./target/release/supercodex --version

# Run the core tests
cargo test -p codex-core --lib
cargo test -p codex-login --lib account_registry
cargo test -p codex-models-manager --lib
```

A full workspace `cargo test` is viable but slow (~40 s plus compile);
prefer the scoped invocations while iterating on fork-specific code.

The Node postinstall is pure stdlib — no install-time `npm install`
needed to edit [`scripts/install.mjs`](codex-cli/scripts/install.mjs)
or [`bin/codex.js`](codex-cli/bin/codex.js).

---

## 6. Maintenance workflow (how the fork stays alive)

Super Codex does not run on a fixed release cadence. Its heartbeat is
upstream's stable releases.

### 6.1 Opening a merge window

When [`openai/codex`](https://github.com/openai/codex) publishes a new
stable tag (`rust-vX.Y.Z`, not an alpha), the maintainer:

```bash
git remote add openai https://github.com/openai/codex.git   # one-off
git fetch openai --tags
git rebase rust-vX.Y.Z
# resolve conflicts — usually around session/*, auth, Op::*
git push origin main --force-with-lease
```

Large upstream refactors (e.g. the `codex.rs → session/*` split in
late Q1 2026) are handled by re-porting the fork's modifications to
the new structure inside the conflict-resolution pass.

### 6.2 Triaging the issue/PR backlog

Inside the same merge window, the maintainer reviews everything that
has accumulated on
[`beltromatti/supercodex`](https://github.com/beltromatti/supercodex)
since the previous window. PRs and proposals that fit one of the
focus areas in [`docs/contributing.md`](docs/contributing.md) § 1
are cherry-picked into the rebased branch. The rest stay open for
the next window.

### 6.3 Cutting the release

```bash
git tag -a super-vX.Y.Z -m "Super Codex X.Y.Z"
git push origin super-vX.Y.Z
```

That tag push is the only manual step. The rest — builds, GitHub
Release assets, npm publish — is what the workflow in section 4
automates.

### 6.4 Rolling the npm token

The workflow authenticates to npm via the `NPM_TOKEN` repo secret,
which must be an **Automation** token (Classic or Granular). Publish
tokens that require a TOTP challenge will fail the publish step with
`EOTP`. To rotate:

```bash
gh secret set NPM_TOKEN --repo beltromatti/supercodex --body "<new-token>"
```

### 6.5 Deprecating old package names

If a package name or version ever needs to be redirected (the fork
has already used `@beltromatti/super-codex` before settling on
`@beltromatti/supercodex`), the standard path is:

```bash
npm deprecate '<old-name>@<range>' "Renamed to <new-name> — run: npm install -g <new-name>"
```

Deprecation is advisory: old installs keep working, new installs get
a warning pointing at the current name.

---

## 7. Files to read next

- [`README.md`](README.md) — vision and disclaimer, user-facing.
- [`docs/contributing.md`](docs/contributing.md) — scope and workflow
  for issues and pull requests.
- [`.github/workflows/release.yml`](.github/workflows/release.yml) —
  the single source of truth for how a tag becomes a release.
- [`codex-cli/scripts/install.mjs`](codex-cli/scripts/install.mjs) —
  the postinstall binary fetcher (40 lines, stdlib-only).
- [`codex-rs/login/src/auth/account_registry.rs`](codex-rs/login/src/auth/account_registry.rs)
  — the multi-account persistence layer.
- [`codex-rs/tui/src/chatwidget.rs`](codex-rs/tui/src/chatwidget.rs)
  — slash-command handlers for `/accounts`, `/addaccount`,
  `/removeaccount`, and the Qwen vLLM prompt view.
