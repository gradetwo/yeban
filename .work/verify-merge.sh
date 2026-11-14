#!/bin/bash
export PATH="$HOME/.cargo/bin:$PATH"
export LIBCLANG_PATH="$HOME/workspace/tools/libclang-noble/usr/lib/llvm-18/lib/libclang.so"
export BINDGEN_EXTRA_CLANG_ARGS="-I$HOME/workspace/tools/libclang-noble/usr/lib/llvm-18/lib/clang/18/include"
export CARGO_TARGET_DIR="$HOME/workspace/tmp-yeban-shared-target"
export CARGO_BUILD_JOBS=1
cd "$HOME/workspace/tmp-wt-yeban-main-merge" || exit 1
echo "== fmt"
cargo fmt --all --check; echo "FMT_EXIT=$?"
echo "== r77 regen (R77_WRITE=1)"
R77_WRITE=1 cargo test -p yeban-dsp --test r77_hard_assertion_audit 2>&1 | grep -E "test result|error|FAILED|panicked|left:|写入|重写"
echo "R77_EXIT=${PIPESTATUS[0]}"
echo "== fault_vocabulary"
cargo test -p yeban-mcp --test fault_vocabulary -- --test-threads=1 2>&1 | grep -E "test result|error|FAILED|panicked|left:|right:"
echo "FAULT_EXIT=${PIPESTATUS[0]}"
echo "VERIFY_MERGE_DONE"
