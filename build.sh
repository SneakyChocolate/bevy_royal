#!/usr/bin/env sh

RUSTFLAGS=-Awarnings cargo build -r --workspace --all-targets
