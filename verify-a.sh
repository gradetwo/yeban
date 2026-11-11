#!/usr/bin/env bash
# A 级（--loudness-target / --true-peak-ceiling）父 agent 亲自验核：rebase 到 d5ddf340 后的完整面。
# 日志落 verify-a.log；每步都跑完，不因一步失败而停（退出码各自记录）。
export PATH="$HOME/.cargo/bin:$PATH"
export LIBCLANG_PATH="$HOME/workspace/tools/libclang-noble/usr/lib/llvm-18/lib/libclang.so"
export BINDGEN_EXTRA_CLANG_ARGS="-I$HOME/workspace/tools/libclang-noble/usr/lib/llvm-18/lib/clang/18/include"
cd ~/workspace/tmp-wt-yeban-loudness-target || exit 1
echo "HEAD=$(git rev-parse HEAD) BASE=$(git merge-base HEAD origin/main)"
cargo fmt --all -- --check; echo "FMT_EXIT=$?"
cargo clippy -p yeban-app --all-targets -- -D warnings; echo "CLIPPY_EXIT=$?"
cargo test -p yeban-app --test export_wav_loudness; echo "LOUDNESS_EXIT=$?"
cargo test -p yeban-app --test cli_contract; echo "CLI_CONTRACT_EXIT=$?"
cargo test -p yeban-app --lib every_action_switch_routes_exactly_as_the_shared_table_says; echo "CENSUS1_EXIT=$?"
cargo test -p yeban-app --lib every_usage_example_that_names_an_action_switch_selects_batch; echo "CENSUS2_EXIT=$?"
python3 scripts/gates/check_feature_alignment.py; echo "GATE1_EXIT=$?"
python3 scripts/gates/check_test_criteria_presence.py; echo "GATE2_EXIT=$?"
python3 scripts/gates/check_golden_manifest.py; echo "GATE3_EXIT=$?"
echo "VERIFY_A_DONE"
