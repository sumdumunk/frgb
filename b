#!/bin/sh
set -e
cd "$(dirname "$0")"

case "${1:-a}" in
    r) cargo build --release ;;
    d) cargo build ;;
    a) cargo build && cargo build --release ;;
    *) echo "Usage: b [r|d|a]  (release/debug/all, default: all)" >&2; exit 1 ;;
esac
