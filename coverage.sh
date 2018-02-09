#!/bin/sh
set -eu

cd "$(dirname "$0")"

RUSTFLAGS="-C link-dead-code"
export RUSTFLAGS

cargo test --no-run

TESTFILE=$(
	find \
		target/debug/ \
		-type f \
		-name "static_http_cache-*" \
	| head -1
)

../kcov/src/kcov \
	--verify \
	--include-path="$PWD" \
	target/cov \
	$TESTFILE
