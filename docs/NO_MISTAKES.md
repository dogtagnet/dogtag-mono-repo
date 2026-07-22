# No-mistakes guardrails

Dogtag's repository configuration keeps the no-mistakes housekeeping steps bounded and check-only.

## What the deterministic commands cover

`commands.lint` runs the Document-commit guard and check-only TypeScript compilation for `@dogtag/standard` and `@dogtag/ui`. These checks do not write formatted files, generate code, or synchronize bindings.

`commands.test` runs the focused guard tests and the existing `@dogtag/standard` Vitest suite. It intentionally does not duplicate the full Cargo, Foundry, circuit, UI, mobile, or Playwright suites. With user intent supplied, no-mistakes v1.40.0 still launches its test evidence agent; a configured test command does not eliminate all agent work.

For that evidence pass, `AGENTS.md` limits the agent to the configured command and the smallest directly relevant checks. Browser or screenshot work is appropriate only when the submitted diff changes that UI. The 15-minute local evidence budget is a prompt and supervision rule, not a hard timeout enforced by no-mistakes.

## Document boundary

The Document agent may update at most 10 documentation files, and only when the submitted branch directly made them stale. It must not edit functional source, tests, workflows, circuits, contracts, generated bindings, or other non-documentation paths, and it must not run write-mode formatters, generators, codegen, or UniFFI/binding synchronization. Work requiring a wider or cross-slice reconciliation is an ask-user finding, not a Document edit.

No-mistakes v1.40.0 has no absolute step timeout, OS-level path allowlist, or built-in file-write budget. The instructions in `.no-mistakes.yaml` and `AGENTS.md` are prompt constraints. The lint-invoked repository guard adds commit-level enforcement: commits whose configured subject begins `no-mistakes(document): ` fail when they touch a non-documentation path, a known generated/codegen path, or more than 10 files.

`auto_fix.document: 0` disables follow-up fix loops only. The initial Document agent may still make and commit fixes, which is why both the prompt boundary and commit guard are needed.

## Trusted-default-branch activation

In v1.40.0, `agent`, `commands`, `document.instructions`, `allow_repo_commands`, and `disable_project_settings` are gate-control settings read from the trusted default branch. The settings added by the guard change do not protect feature-branch runs until the change reaches the default branch; unsafe per-branch commands remain disabled.

Until then, use the hard bypass for a new run:

```sh
no-mistakes axi run --skip=document --intent "<what the user set out to accomplish>"
```

`no-mistakes rerun` has no `--skip` flag, so it cannot express this bypass by itself.
