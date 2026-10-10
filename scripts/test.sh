#!/usr/bin/env bash
# Full test run. Tests load PRE-BUILT .so files, so always build first.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.local/share/solana/install/active_release/bin:$HOME/.cargo/bin:$PATH"
# Bootstrap note: tests/mock-game reads idls/lobby.json via declare_program!. If you change an ix
# that mock-game calls, the first build may fail on the stale IDL → build once without the
# mock_game dev-dependency, copy target/idl/lobby.json → idls/, then run this script again.
anchor build
cp target/idl/lobby.json idls/lobby.json
if [ -d tests/mock-game ]; then
  cargo build-sbf --manifest-path tests/mock-game/Cargo.toml --sbf-out-dir target/deploy
fi
cargo test -p lobby "$@"
