#!/usr/bin/env bash
# SDK conformance: the plugins `hello`, `shop` (a UI), `notify` (notifications) and `jobsdemo` (background jobs, in
# sdk/conformance/jobs) are declared the same way in every SDK (sdk/conformance/<plugin>/<lang>/); each is built, run
# with --describe, and its output, canonicalized (object keys sorted by `jq -S .`, arrays as printed), must equal
# sdk/conformance/<plugin>/descriptor.json.
#   ./scripts/sdk-conformance.sh [--strict] [rust] [ts] [csharp]
# Default: all three languages; a language whose toolchain is missing is skipped with a note. With --strict (or
# CONFORMANCE_REQUIRE_ALL=1), as in CI, a missing toolchain or conformance plugin fails the run instead.
set -euo pipefail
cd "$(dirname "$0")/.."
STRICT=${CONFORMANCE_REQUIRE_ALL:-0}
[[ "$STRICT" == 1 || "$STRICT" == true ]] && STRICT=1 || STRICT=0
args=()
for a in "$@"; do
  case "$a" in
    --strict) STRICT=1 ;;
    *) args+=("$a") ;;
  esac
done
set -- ${args[@]+"${args[@]}"}
FIXTURE=sdk/conformance/hello/descriptor.json
SHOP=sdk/conformance/shop/descriptor.json
NOTIFY=sdk/conformance/notify/descriptor.json
JOBS=sdk/conformance/jobs/descriptor.json
LANGS=("$@")
[[ ${#LANGS[@]} -eq 0 ]] && LANGS=(rust ts csharp)
command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }
fail=0
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

skip() { # why: a skip, or a failure in strict mode
  if [[ $STRICT == 1 ]]; then echo "FAIL $1 (strict)"; fail=1; else echo "skip $1"; fi
}

compare() { # lang, command... (FIXTURE: the expected descriptor)
  local lang=$1; shift
  if ! "$@" --describe >"$tmp/$lang.raw" 2>"$tmp/$lang.err"; then
    echo "FAIL $lang: --describe exited non-zero"; cat "$tmp/$lang.err"; fail=1; return
  fi
  if ! jq -S . "$tmp/$lang.raw" >"$tmp/$lang.json"; then
    echo "FAIL $lang: --describe did not print JSON"; fail=1; return
  fi
  if diff -u "$FIXTURE" "$tmp/$lang.json"; then
    echo "ok   $lang"
  else
    echo "FAIL $lang: the descriptor differs from $FIXTURE (diff above)"; fail=1
  fi
}

query() { # lang, command...: the SDK's test of the conformance query's tree
  local lang=$1; shift
  if "$@" >"$tmp/$lang-query.out" 2>&1; then
    echo "ok   $lang-query"
  else
    echo "FAIL $lang-query: the query tree differs from sdk/conformance/query/revenue.json"; cat "$tmp/$lang-query.out"; fail=1
  fi
}

for lang in "${LANGS[@]}"; do
  case "$lang" in
    rust)
      cargo build -q -p celobus-conformance-hello -p celobus-conformance-shop -p celobus-conformance-notify -p celobus-conformance-jobs
      compare rust target/debug/hello
      FIXTURE=$SHOP compare rust-shop target/debug/shop
      FIXTURE=$NOTIFY compare rust-notify target/debug/notify
      FIXTURE=$JOBS compare rust-jobs target/debug/jobsdemo
      query rust cargo test -q -p celobus-sdk --lib the_conformance_query_is_the_shared_tree
      ;;
    ts)
      if command -v npm >/dev/null; then
        (cd sdk/node && npm ci --silent && npm run --silent build) >/dev/null
        (cd sdk/conformance/hello/ts && npm ci --silent && npm run --silent build) >/dev/null
        compare ts node sdk/conformance/hello/ts/dist/main.js
        if [[ -d sdk/conformance/shop/ts ]]; then
          (cd sdk/conformance/shop/ts && npm ci --silent && npm run --silent build) >/dev/null
          FIXTURE=$SHOP compare ts-shop node sdk/conformance/shop/ts/dist/main.js
        else
          skip "ts-shop: sdk/conformance/shop/ts is missing"
        fi
        (cd sdk/conformance/notify/ts && npm ci --silent && npm run --silent build) >/dev/null
        FIXTURE=$NOTIFY compare ts-notify node sdk/conformance/notify/ts/dist/main.js
        (cd sdk/conformance/jobs/ts && npm ci --silent && npm run --silent build) >/dev/null
        FIXTURE=$JOBS compare ts-jobs node sdk/conformance/jobs/ts/dist/main.js
        query ts node --test --test-name-pattern "the conformance query" sdk/node/test/query.test.mjs
      else
        skip "ts: node and npm are not installed"
      fi
      ;;
    csharp)
      if command -v dotnet >/dev/null; then
        dotnet build -c Release --nologo -v q sdk/conformance/hello/csharp >/dev/null
        compare csharp dotnet sdk/conformance/hello/csharp/bin/Release/net10.0/Hello.dll
        if [[ -d sdk/conformance/shop/csharp ]]; then
          dotnet build -c Release --nologo -v q sdk/conformance/shop/csharp >/dev/null
          FIXTURE=$SHOP compare csharp-shop dotnet sdk/conformance/shop/csharp/bin/Release/net10.0/Shop.dll
        else
          skip "csharp-shop: sdk/conformance/shop/csharp is missing"
        fi
        dotnet build -c Release --nologo -v q sdk/conformance/notify/csharp >/dev/null
        FIXTURE=$NOTIFY compare csharp-notify dotnet sdk/conformance/notify/csharp/bin/Release/net10.0/Notify.dll
        dotnet build -c Release --nologo -v q sdk/conformance/jobs/csharp >/dev/null
        FIXTURE=$JOBS compare csharp-jobs dotnet sdk/conformance/jobs/csharp/bin/Release/net10.0/Jobsdemo.dll
        query csharp dotnet test --nologo -v q sdk/dotnet/Celobus.Sdk.Tests --filter TheConformanceQueryIsTheSharedTree
      else
        skip "csharp: dotnet is not installed"
      fi
      ;;
    *) echo "unknown language $lang (rust, ts, csharp)" >&2; exit 1 ;;
  esac
done
exit $fail
