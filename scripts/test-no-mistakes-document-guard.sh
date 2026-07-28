#!/usr/bin/env bash
# Focused behavior and v1.40.0 repo-config semantics checks for the Document guard.

set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
guard="$repo_root/scripts/check-no-mistakes-document-guard.sh"
config="$repo_root/.no-mistakes.yaml"
agents="$repo_root/AGENTS.md"
guide="$repo_root/docs/NO_MISTAKES.md"
test_root=$(mktemp -d "${TMPDIR:-/tmp}/dogtag-nmguard.XXXXXX")
trap 'rm -rf "$test_root"' EXIT

fail() {
  printf 'not ok - %s\n' "$*" >&2
  exit 1
}

pass() {
  printf 'ok - %s\n' "$*"
}

new_repo() {
  local name=$1
  local repo="$test_root/$name"
  mkdir -p "$repo"
  git -C "$repo" init -q -b main
  git -C "$repo" config user.name 'Dogtag guard test'
  git -C "$repo" config user.email 'guard-test@dogtag.invalid'
  printf 'base\n' > "$repo/README.md"
  git -C "$repo" add README.md
  git -C "$repo" commit -q -m 'test: base'
  git -C "$repo" switch -q -c feature
  printf '%s\n' "$repo"
}

expect_pass() {
  local name=$1
  local repo=$2
  if ! output=$(cd "$repo" && "$guard" main HEAD 2>&1); then
    fail "$name (unexpected failure: $output)"
  fi
  pass "$name"
}

expect_fail() {
  local name=$1
  local expected=$2
  local repo=$3
  if output=$(cd "$repo" && "$guard" main HEAD 2>&1); then
    fail "$name (unexpected pass: $output)"
  fi
  case "$output" in
    *"$expected"*) pass "$name" ;;
    *) fail "$name (missing '$expected' in: $output)" ;;
  esac
}

# no-mistakes v1.40.0 RepoConfig fields and types, including the trusted-control
# switches and commit template used by the guard. This is deliberately static:
# running the no-mistakes pipeline is forbidden until firstmate reviews this PR.
ruby --disable-gems - "$config" "$agents" "$guide" <<'RUBY'
require "yaml"

config = YAML.safe_load(File.read(ARGV.fetch(0)), permitted_classes: [], aliases: false)
raise "agent" unless config["agent"] == "claude"
raise "allow_repo_commands" unless config["allow_repo_commands"] == false
raise "disable_project_settings" unless config["disable_project_settings"] == false
raise "lint command" unless config.dig("commands", "lint").is_a?(String) &&
  config.dig("commands", "lint").include?("check-no-mistakes-document-guard.sh") &&
  config.dig("commands", "lint").include?("tsc -p tsconfig.json --noEmit") &&
  config.dig("commands", "lint").include?("@dogtag/ui typecheck")
raise "test command" unless config.dig("commands", "test").is_a?(String) &&
  config.dig("commands", "test").include?("test-no-mistakes-document-guard.sh") &&
  config.dig("commands", "test").include?("@dogtag/standard test")
# Both commands must satisfy their own prerequisites. Dropping these turns a fresh worktree's
# `Command "tsc"/"vitest" not found` back into something that reads like a flaky code finding,
# and approving past it would sign off a step that never executed. lint needs --sdk-dist
# because @dogtag/ui resolves @dogtag/standard's types from the SDK's gitignored dist/.
# The needles carry the "scripts/" segment deliberately: a bare "ensure-ts-prereqs.sh" is a
# substring of the neighbouring "scripts/test-ensure-ts-prereqs.sh" invocation, so it would
# pass even after the prerequisite call itself were deleted - an assertion that cannot fail.
raise "lint prerequisites" unless config.dig("commands", "lint").include?("scripts/ensure-ts-prereqs.sh --sdk-dist")
raise "test prerequisites" unless config.dig("commands", "test").include?("scripts/ensure-ts-prereqs.sh")
# The document guard needs no node_modules and fails closed on a dirty worktree, so it must
# stay ahead of any install.
raise "guard runs before prerequisites" unless
  config.dig("commands", "lint").index("check-no-mistakes-document-guard.sh") <
    config.dig("commands", "lint").index("scripts/ensure-ts-prereqs.sh")
raise "write-mode lint" if config.dig("commands", "lint").match?(/(?:^|\s)(?:--write|-w)(?:\s|$)/)
raise "auto_fix" unless %w[review document lint].all? { |step| config.dig("auto_fix", step) == 0 }
raise "commit template" unless config.dig("commit", "fix_message") ==
  "no-mistakes({{.Step}}): {{.Summary}}"

instructions = config.dig("document", "instructions")
%w[10 non-documentation cross-slice UniFFI ask-user functional tests workflows circuits contracts].each do |term|
  raise "document.instructions missing #{term}" unless instructions.include?(term)
end

[File.read(ARGV.fetch(1)), File.read(ARGV.fetch(2))].each do |policy|
  raise "missing mandatory Document skip" unless policy.include?("--skip=document")
  raise "missing upstream budget condition" unless policy.include?("enforced step/file budget")
  raise "missing rerun warning" unless policy.include?("rerun")
  raise "guard represented as runtime cap" unless policy.include?("not a runtime cap")
end
RUBY
pass '.no-mistakes.yaml and skip policy match v1.40.0 semantics'

repo=$(new_repo unstaged-dirty)
printf 'unstaged change\n' >> "$repo/README.md"
expect_fail 'unstaged worktree changes fail closed' 'worktree is dirty' "$repo"

repo=$(new_repo staged-dirty)
mkdir -p "$repo/docs"
printf 'staged change\n' > "$repo/docs/staged.md"
git -C "$repo" add docs/staged.md
expect_fail 'staged worktree changes fail closed' 'worktree is dirty' "$repo"

repo=$(new_repo untracked-dirty)
printf 'untracked change\n' > "$repo/untracked.txt"
expect_fail 'untracked worktree changes fail closed' 'worktree is dirty' "$repo"

repo=$(new_repo ordinary-commit)
mkdir -p "$repo/src"
printf 'functional change\n' > "$repo/src/lib.rs"
git -C "$repo" add src/lib.rs
git -C "$repo" commit -q -m 'feat: ordinary source change'
expect_pass 'ordinary feature commits are outside the Document guard' "$repo"

repo=$(new_repo ten-docs)
mkdir -p "$repo/docs"
for n in $(seq 1 10); do
  printf 'doc %s\n' "$n" > "$repo/docs/doc-$n.md"
done
git -C "$repo" add docs
git -C "$repo" commit -q -m 'no-mistakes(document): update bounded docs'
expect_pass 'Document commit may touch exactly ten documentation files' "$repo"

repo=$(new_repo eleven-docs)
mkdir -p "$repo/docs"
for n in $(seq 1 11); do
  printf 'doc %s\n' "$n" > "$repo/docs/doc-$n.md"
done
git -C "$repo" add docs
git -C "$repo" commit -q -m 'no-mistakes(document): update too many docs'
expect_fail 'Document commit over budget fails' 'touches 11 files; budget is 10' "$repo"

repo=$(new_repo source-path)
mkdir -p "$repo/src"
printf 'functional change\n' > "$repo/src/lib.rs"
git -C "$repo" add src/lib.rs
git -C "$repo" commit -q -m 'no-mistakes(document): rewrite source'
expect_fail 'Document commit touching source fails' 'touches non-documentation path: src/lib.rs' "$repo"

repo=$(new_repo forbidden-slice)
mkdir -p "$repo/circuits"
printf 'circuit notes\n' > "$repo/circuits/README.md"
git -C "$repo" add circuits/README.md
git -C "$repo" commit -q -m 'no-mistakes(document): update circuit notes'
expect_fail 'Document commit touching a forbidden slice fails even for Markdown' 'touches forbidden workflow/test/circuit/contract path' "$repo"

repo=$(new_repo generated-binding)
mkdir -p "$repo/apps/ios/DogTag"
printf 'generated binding\n' > "$repo/apps/ios/DogTag/dogtag_standard.swift"
git -C "$repo" add apps/ios/DogTag/dogtag_standard.swift
git -C "$repo" commit -q -m 'no-mistakes(document): synchronize bindings'
expect_fail 'Document commit touching generated bindings fails' 'touches generated binding/codegen output' "$repo"
