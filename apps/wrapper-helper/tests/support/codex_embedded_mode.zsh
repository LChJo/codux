#!/bin/zsh
set -eu

# Load the actual launch function without running the wrapper entry point.
eval "$(sed -n '/^apply_codex_embedded_mode() {/,/^codex_allows_additional_writable_roots() {/p' "$1" | sed '$d')"
real_bin="$2"
search_path="$PATH"
log_line() { :; }
checked=0

check_launch() {
  local add="$1"
  shift
  launch_args=("$@")
  local -a expected=("$@")
  if [[ "$CODEX_TEST_HELP" == modern && "$add" == yes ]]; then
    expected=(--no-daemon "${expected[@]}")
  fi
  apply_codex_embedded_mode
  [[ ${#launch_args[@]} -eq ${#expected[@]} ]] || { print -u2 -- "wrong argument count: $CODEX_TEST_HELP $*"; exit 1; }
  local i
  for ((i=1; i<=${#expected[@]}; i++)); do
    [[ "${launch_args[i]}" == "${expected[i]}" ]] || { print -u2 -- "changed argument $i: $CODEX_TEST_HELP $*"; exit 1; }
  done
  checked=$((checked+1))
}

for mode in modern old fail; do
  export CODEX_TEST_HELP="$mode"
  check_launch yes --enable hooks
  check_launch yes -c 'developer_instructions="中文"' --profile work 'Initial "task"'
  check_launch yes '--config=model_reasoning_effort="high"' resume --last
  check_launch yes '-cmodel="mine"' fork session-id
  check_launch yes --disable=plugins -- --remote
  check_launch yes --search
  check_launch yes --enable hooks --profile exec 'Initial task'
  check_launch no --enable hooks --no-daemon
  check_launch no --enable hooks --remote ws://localhost:1234
  check_launch no --enable hooks --remote=ws://localhost:1234
  check_launch no --enable hooks --profile work agents
  check_launch no --enable hooks exec 'Initial task'
  check_launch no --enable hooks review
  check_launch no --enable hooks app-server
  check_launch no --enable hooks debug prompt-input
  check_launch no --enable hooks --help
  check_launch no --model mine 'Initial task'
  check_launch no -- --enable hooks
done
print -r -- "checked=$checked"
