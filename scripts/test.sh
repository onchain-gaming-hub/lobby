#!/usr/bin/env bash
# Full test run. Tests load PRE-BUILT .so files, so always build first.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.local/share/solana/install/active_release/bin:$HOME/.cargo/bin:$PATH"
anchor build
cp target/idl/lobby.json idls/lobby.json
if [ -d tests/mock-game ]; then
  cargo build-sbf --manifest-path tests/mock-game/Cargo.toml --sbf-out-dir target/deploy
fi
cargo test -p lobby "$@"
