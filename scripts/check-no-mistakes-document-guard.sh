#!/usr/bin/env bash
# Fail closed when a no-mistakes Document commit exceeds Dogtag's documentation scope.

set -euo pipefail

readonly DOCUMENT_COMMIT_PREFIX='no-mistakes(document): '
readonly DOCUMENT_FILE_BUDGET=10

fail() {
  printf 'no-mistakes document guard: %s\n' "$*" >&2
  exit 1
}

is_documentation_path() {
  case "$1" in
    docs/*|*.md|*.mdx|*/README|*/README.*|README|README.*|CONTRIBUTING|CONTRIBUTING.*|CHANGELOG|CHANGELOG.*|SECURITY|SECURITY.*|CODE_OF_CONDUCT|CODE_OF_CONDUCT.*|LICENSE|LICENSE.*|NOTICE|NOTICE.*)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

is_generated_binding_or_codegen_path() {
  case "$1" in
    apps/android/app/src/main/java/uniffi/*|\
    apps/ios/DogTag/dogtag_standard.swift|\
    circuits/build/*|\
    circuits/poseidon-vectors.json|\
    apps/android/app/src/main/assets/testvectors.json|\
    apps/ios/DogTag/testvectors.json|\
    packages/dogtag-standard-ts/testvectors.json|\
    contracts/out/*|\
    contracts/cache/*|\
    */generated/*|\
    *.generated.*)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

is_forbidden_document_slice_path() {
  case "$1" in
    .github/workflows/*|circuits/*|contracts/*|test/*|tests/*|*/test/*|*/tests/*|*.test.*|*.spec.*)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) || fail 'not inside a Git repository'
cd "$repo_root"

dirty_state=$(git status --porcelain=v1 --untracked-files=all --ignore-submodules=none) || fail 'cannot inspect worktree state'
if [ -n "$dirty_state" ]; then
  fail 'worktree is dirty; commit or remove all staged, unstaged, untracked, and submodule changes before lint'
fi

head_ref=${2:-HEAD}
git rev-parse --verify --quiet "${head_ref}^{commit}" >/dev/null || fail "head ref is not a commit: $head_ref"

if [ "$#" -ge 1 ]; then
  base_ref=$1
else
  base_ref=''
  for candidate in refs/remotes/origin/main origin/main refs/heads/main main; do
    if git rev-parse --verify --quiet "${candidate}^{commit}" >/dev/null; then
      base_ref=$candidate
      break
    fi
  done
  [ -n "$base_ref" ] || fail 'cannot resolve the trusted default branch (expected origin/main)'
fi

git rev-parse --verify --quiet "${base_ref}^{commit}" >/dev/null || fail "base ref is not a commit: $base_ref"
merge_base=$(git merge-base "$base_ref" "$head_ref") || fail "cannot find a merge base for $base_ref and $head_ref"

document_commits=0
while IFS= read -r commit; do
  [ -n "$commit" ] || continue
  subject=$(git show -s --format=%s "$commit")
  case "$subject" in
    "$DOCUMENT_COMMIT_PREFIX"*) ;;
    *) continue ;;
  esac

  document_commits=$((document_commits + 1))
  file_count=0
  while IFS= read -r path; do
    [ -n "$path" ] || continue
    file_count=$((file_count + 1))

    if is_generated_binding_or_codegen_path "$path"; then
      fail "Document commit $commit touches generated binding/codegen output: $path"
    fi
    if is_forbidden_document_slice_path "$path"; then
      fail "Document commit $commit touches forbidden workflow/test/circuit/contract path: $path"
    fi
    if ! is_documentation_path "$path"; then
      fail "Document commit $commit touches non-documentation path: $path"
    fi
  done < <(git diff-tree --root --no-commit-id --name-only -r --diff-filter=ACDMRTUXB "$commit")

  if [ "$file_count" -gt "$DOCUMENT_FILE_BUDGET" ]; then
    fail "Document commit $commit touches $file_count files; budget is $DOCUMENT_FILE_BUDGET"
  fi
done < <(git rev-list --reverse "${merge_base}..${head_ref}")

printf 'no-mistakes document guard: checked %d Document commit(s)\n' "$document_commits"
