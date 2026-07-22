# No-mistakes guardrails

Dogtag uses targeted no-mistakes commands plus defense-in-depth checks. They do not impose a runtime cap on an agent step.

## Run policy: skip Document

Every new Dogtag validation run must skip Document:

```sh
no-mistakes axi run --skip=document --intent "<what the user set out to accomplish>"
```

This policy remains in force until upstream no-mistakes provides an enforced step/file budget and Dogtag deliberately revises this rule. Do not use bare `axi run` or `no-mistakes rerun` while the policy is active; `rerun` has no `--skip` flag, so start a fresh run with the command above.

## What the deterministic commands cover

`commands.lint` runs the Document-commit guard and check-only TypeScript compilation for `@dogtag/standard` and `@dogtag/ui`. These checks do not write formatted files, generate code, or synchronize bindings.

`commands.test` runs the focused guard tests and the existing `@dogtag/standard` Vitest suite. It intentionally does not duplicate the full Cargo, Foundry, circuit, UI, mobile, or Playwright suites. With user intent supplied, no-mistakes v1.40.0 still launches its test evidence agent; a configured test command does not eliminate all agent work.

For that evidence pass, `AGENTS.md` limits the agent to the configured command and the smallest directly relevant checks. Browser or screenshot work is appropriate only when the submitted diff changes that UI. The 15-minute local evidence budget is a prompt and supervision rule, not a hard timeout enforced by no-mistakes.

## Accidental raw-run boundary

If someone accidentally starts a raw run without `--skip=document`, the Document agent may update at most 10 documentation files, and only when the submitted branch directly made them stale. It must not edit functional source, tests, workflows, circuits, contracts, generated bindings, or other non-documentation paths, and it must not run write-mode formatters, generators, codegen, or UniFFI/binding synchronization. Work requiring a wider or cross-slice reconciliation is an ask-user finding, not a Document edit.

No-mistakes v1.40.0 has no absolute step timeout, OS-level path allowlist, or built-in file-write budget. The instructions in `.no-mistakes.yaml` and `AGENTS.md` are prompt constraints. The lint-invoked repository guard runs only after earlier stages, so it is defense-in-depth and not a runtime cap: it rejects any dirty gate worktree, then rejects commits whose configured subject begins `no-mistakes(document): ` when they touch a non-documentation path, a known generated/codegen path, or more than 10 files. Rejecting staged, unstaged, untracked, or dirty-submodule state prevents an unfinished sweep from evading commit-prefix inspection.

`auto_fix.document: 0` disables follow-up fix loops only. The initial Document agent may still make and commit fixes, which is why both the prompt boundary and commit guard are needed.

## Trusted-default-branch activation

In v1.40.0, `agent`, `commands`, `document.instructions`, `allow_repo_commands`, and `disable_project_settings` are gate-control settings read from the trusted default branch. The settings added by the guard change do not protect feature-branch runs until the change reaches the default branch; unsafe per-branch commands remain disabled. The mandatory `--skip=document` policy applies both before and after activation. Once active, the config and guard constrain accidental raw runs that omit the skip.
