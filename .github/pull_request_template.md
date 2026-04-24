<!--
Thanks for sending a pull request to Super Codex.

Before opening it, please read the contributing guide:
https://github.com/beltromatti/supercodex/blob/main/docs/contributing.md

In particular note that Super Codex is an unofficial fork that re-merges
from openai/codex on every upstream stable release. Changes that belong
to the upstream project itself should go to https://github.com/openai/codex
— they will reach Super Codex automatically at the next merge window.

This PR is a good fit for Super Codex if it touches one of the focus areas
described in the contributing guide (multi-account ChatGPT management,
additional self-hosted / non-OpenAI provider support, or maintenance of
the fork itself: release workflow, branding, postinstall, docs).

Delete everything above this line once you've read it, then fill in the
sections below.
-->

## Summary
<!-- One or two sentences on what this PR changes and why. -->

## Focus area
<!-- Tick the one that applies. -->
- [ ] Multi-account ChatGPT / auth
- [ ] Provider support (Qwen vLLM, other non-OpenAI providers)
- [ ] Fork maintenance (release workflow, branding, postinstall, docs)
- [ ] Bug fix specific to Super Codex
- [ ] Other (explain below)

## Details
<!-- Explain the change, decisions you made, and any tradeoffs. Link to issues. -->

## Testing
<!-- How did you verify this works? `cargo test`, manual TUI walk-through, etc. -->

## Checklist
- [ ] I have read [`docs/contributing.md`](../docs/contributing.md).
- [ ] The change is specific to Super Codex and does not belong upstream in `openai/codex`.
- [ ] `cargo check --workspace --all-targets` passes on `codex-rs/`.
- [ ] User-facing changes are reflected in `README.md` where appropriate.
