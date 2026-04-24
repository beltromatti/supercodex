# Contributing to Super Codex

Super Codex is an **unofficial, community-driven fork** of
[`openai/codex`](https://github.com/openai/codex). It is maintained by an
independent developer on personal time. There is no company, no SLA, no
paid support. That context shapes everything below.

This page covers:

- the scope of what belongs in Super Codex (and what does not),
- how the fork is kept in sync with upstream,
- how contributions flow through that sync cycle,
- and what to expect as a contributor or user filing an issue.

Reading this file in full before opening an issue or a pull request is
appreciated — it will save everyone time.

---

## 1. Where the project is going

Super Codex's purpose is to serve users whose needs fall outside the
happy-path that upstream OpenAI Codex is optimised for. The project is
currently evolving along **two active focus areas**:

### 1.1 Multi-account ChatGPT support

- CLI commands: `/accounts` (list + switch in one view), `/addaccount`, `/removeaccount`.
- Automatic rotation of saved ChatGPT accounts when the active one hits a
  usage limit mid-turn, so the running task continues without manual
  intervention.
- Persistent account registry on disk, with clean separation between the
  currently active auth and the saved alternatives.

### 1.2 Broader provider support

- First-class integration with a **self-hosted vLLM server** running
  Qwen3-VL-32B-Instruct-AWQ (out of the box, with a runtime prompt for the
  server URL).
- A generalisable pattern that other self-hosted / non-OpenAI providers
  can plug into in future.

Work that helps either of these two tracks is likely to be accepted and
is what maintainer attention gravitates toward. Work outside those tracks
is usually a better fit for the upstream project — see section 2.

### 1.3 Fork maintenance (implicit third track)

Super Codex also maintains a small set of infrastructure that is its own
and will never come from upstream:

- the CLI binary rebrand (`supercodex`),
- the npm package `@beltromatti/supercodex` and its postinstall installer,
- the release workflow in `.github/workflows/release.yml`,
- the Super Codex splash, README, banner, and update-checker URL.

PRs that keep that surface clean are welcome.

---

## 2. Where issues and PRs should actually go

Super Codex is a thin layer over upstream. The vast majority of behaviour
in the binary — the agent loop, model catalog, sandboxing, tools,
approvals, editing UX, MCP, plugins, the whole TUI — lives in
`openai/codex` and comes into Super Codex on the next merge window.

### Open upstream in `openai/codex` if…
- the bug reproduces on the equivalent upstream release, or
- the feature you want is about the core agent, approvals, sandboxing,
  editing, TUI rendering, model catalog entries, plugin system, MCP, or
  anything that is the same in Codex itself.

Those fixes and features reach Super Codex automatically — see section 3.

### Open here in `beltromatti/supercodex` if…
- the bug is specific to one of the fork's focus areas in section 1,
- the bug is in the fork's own maintenance surface (section 1.3), or
- you have a concrete proposal for one of the focus areas.

When in doubt, open it here — the maintainer will redirect upstream if
that's the right place.

---

## 3. Maintenance and update cadence

Super Codex does not live on a fixed release schedule. Its cadence is
**driven by upstream**:

1. Every time **OpenAI Codex ships a new stable release** (tag
   `rust-vX.Y.Z` on `openai/codex`, not an alpha), the Super Codex
   maintainer opens a merge window.
2. During that window the maintainer:
   - rebases the fork's feature commits on top of the new upstream tag,
   - resolves any conflicts introduced by upstream refactors,
   - reviews the issues and pull requests that have accumulated on
     `beltromatti/supercodex` since the previous merge window,
   - pulls in the ones that fit the focus areas from section 1, and
   - ships a new Super Codex release as
     `super-vX.Y.Z` (matching the upstream version the fork is
     based on, with a `+N` suffix for fork-only follow-up patches when
     needed).
3. That `super-vX.Y.Z` tag triggers the release workflow, which builds
   the binary for macOS arm64, Linux x64 and Windows x64 and publishes
   the npm package `@beltromatti/supercodex` with a postinstall script
   that downloads the matching binary from the GitHub Release.

**Practical consequence for contributors**: your issue or PR will not
get an immediate reply. It will be read and triaged the next time a
merge window opens. If you want to accelerate that, the best thing you
can do is pin your PR to one of the focus areas, keep it small, and
include a clear test plan.

---

## 4. How to contribute

### 4.1 Filing an issue
- Search existing issues first (including closed). Upvote with a 👍
  instead of opening a duplicate.
- Use the **Bug Report** template if something is broken.
- If you're unsure whether the bug is fork-specific or upstream, tick
  "I'm not sure" in the template — the maintainer will move it.
- For feature ideas, open a regular issue with a clear problem
  statement and, if you have one, a sketch of the API / UX.

### 4.2 Opening a pull request

Before coding, please:
- read section 1 to check your change fits a focus area,
- confirm the change does not belong in upstream `openai/codex`
  (section 2).

Then:
1. Fork the repository and branch from `main`.
2. Keep the change small and scoped. One PR per concern.
3. Run `cargo check --workspace --all-targets` inside `codex-rs/`.
4. If the change touches user-facing strings, update the relevant docs
   and `README.md`.
5. Fill out the PR template honestly. The "Focus area" checkbox matters.
6. Do not force-push after review starts — just add follow-up commits.

### 4.3 What gets merged

Super Codex PRs are merged when they:
- align with a focus area in section 1,
- come with a clear problem statement and a minimal reproducer or
  test,
- do not silently alter upstream behaviour that users rely on, and
- the maintainer can read and reason about end-to-end in one sitting.

Large refactors, speculative reshuffles, or PRs that rewrite upstream
modules will typically be redirected to `openai/codex`.

---

## 5. What not to send here

- **Security issues in the upstream agent/sandbox code.** Report those
  to OpenAI directly, not here. The fork has no private security channel
  and the fix belongs upstream anyway.
- **Code of Conduct or legal matters.** Super Codex inherits the
  upstream Apache-2.0 license and offers no warranty (see the README's
  disclaimer). For questions about OpenAI's terms of service when using
  multi-account features, consult those terms yourself before enabling
  them.
- **Support questions.** Super Codex does not provide user support. For
  help running the official Codex, ask upstream.

---

Thanks for reading. If you made it this far, you already know more about
this project than 95 % of the people opening issues. Your contribution
will be read with care.
