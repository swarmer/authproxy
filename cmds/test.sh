#!/usr/bin/env bash
set -e
cargo fmt
cmds/check.sh
cargo test
