#!/bin/sh
set -e
cd "$(dirname "$0")"
cargo clippy --workspace --all-targets -- -D warnings
