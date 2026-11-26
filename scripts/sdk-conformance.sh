#!/usr/bin/env bash
# SDK conformance: the plugins `hello`, `shop` (a UI), `notify` (notifications), `jobsdemo` (background jobs,
# analytic views and the Data Lake, in sdk/conformance/jobs) and `prefs` (settings, in sdk/conformance/settings; with
# `--form` it lays its settings form out by hand and checks each change, and must equal descriptor-form.json there) are
# declared the same way in every SDK (sdk/conformance/<plugin>/<lang>/);
# each is built, run with --describe, and its output, canonicalized (object keys sorted by `jq -S .`, arrays as
# printed), must equal sdk/conformance/<plugin>/descriptor.json. Each SDK's unit tests then build the conformance query
# (sdk/conformance/query/revenue.json) and must send that tree, from the analytic engine (`engine = ANALYTIC`) too, and
# the conformance queries over jobsdemo's views on the analytic engine: `recent_task` with a time bucket and a relative
# day (sdk/conformance/query/weekly.json), `long_estimate` with a decimal column compared with a fractional value, sent
# as its text (sdk/conformance/query/estimates.json; the view itself compares one in its declaration). The lake leg: a
# fake platform (sdk/conformance/jobs/rust/tests/lake.rs) runs each SDK's jobsdemo as a process, sends it the requests
# and job runs of sdk/conformance/jobs/lake.json and checks every lake write chunk (table, mode, filter, key, write,
# last, rows), the answers, the lake query (engine LAKE, the tree of sdk/conformance/query/lake.json), the refreshes of
# the models (table, partition filter, the refusals' statuses) and the history reads (whole and per day); it needs
# cargo for every language. jobsdemo's descriptor holds a model of each kind of source, two histories and an entity
# stored in OLAP with its load; the leg also requests the load, follows its ticket (a status, a wait) and maps every
# refusal and failure code of loads.
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
SETTINGS=sdk/conformance/settings/descriptor.json
SETTINGS_FORM=sdk/conformance/settings/descriptor-form.json
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

query() { # name, pattern its output must show (the tests ran), command...: the SDK's test of the conformance query
  local name=$1 ran=$2; shift 2
  if "$@" >"$tmp/$name.out" 2>&1 && grep -Eq "$ran" "$tmp/$name.out"; then
    echo "ok   $name"
  else
    echo "FAIL $name: the query or its engine differs from its file in sdk/conformance/query (or no test ran)"
    cat "$tmp/$name.out"; fail=1
  fi
}

lake() { # name, the plugin's command: the lake leg against that SDK's jobsdemo
  local name=$1; shift
  if ! command -v cargo >/dev/null; then skip "$name: the lake leg's fake platform needs cargo"; return; fi
  query "$name" '1 passed' env CONFORMANCE_PLUGIN="$*" cargo test -q -p celobus-conformance-jobs --test lake
}

for lang in "${LANGS[@]}"; do
  case "$lang" in
    rust)
      cargo build -q -p celobus-conformance-hello -p celobus-conformance-shop -p celobus-conformance-notify -p celobus-conformance-jobs \
        -p celobus-conformance-settings
      compare rust target/debug/hello
      FIXTURE=$SHOP compare rust-shop target/debug/shop
      FIXTURE=$NOTIFY compare rust-notify target/debug/notify
      FIXTURE=$JOBS compare rust-jobs target/debug/jobsdemo
      FIXTURE=$SETTINGS compare rust-settings target/debug/prefs
      FIXTURE=$SETTINGS_FORM compare rust-settings-form target/debug/prefs --form
      query rust-query '1 passed' cargo test -q -p celobus-sdk --lib the_conformance_query_is_the_shared_tree
      query rust-analytic '1 passed' cargo test -q -p celobus-sdk --lib an_analytic_read_names_its_engine
      query rust-view-query '1 passed' cargo test -q -p celobus-conformance-jobs the_conformance_view_query_is_the_shared_tree
      query rust-decimal-query '1 passed' cargo test -q -p celobus-conformance-jobs the_conformance_decimal_query_is_the_shared_tree
      query rust-lake-query '1 passed' cargo test -q -p celobus-conformance-jobs the_conformance_lake_query_is_the_shared_tree
      lake rust-lake "$PWD/target/debug/jobsdemo"
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
        (cd sdk/conformance/settings/ts && npm ci --silent && npm run --silent build) >/dev/null
        FIXTURE=$SETTINGS compare ts-settings node sdk/conformance/settings/ts/dist/main.js
        FIXTURE=$SETTINGS_FORM compare ts-settings-form node sdk/conformance/settings/ts/dist/main.js --form
        query ts-query 'pass 1$' node --test --test-name-pattern "the conformance query is the shared tree" sdk/node/test/query.test.mjs
        query ts-analytic 'pass 1$' node --test --test-name-pattern "the conformance query reads the analytic engine" sdk/node/test/query.test.mjs
        query ts-view-query 'pass 1$' node --test --test-name-pattern "the conformance view query is the shared tree" sdk/node/test/views.test.mjs
        query ts-decimal-query 'pass 1$' node --test --test-name-pattern "the conformance decimal query is the shared tree" sdk/node/test/views.test.mjs
        lake ts-lake node "$PWD/sdk/conformance/jobs/ts/dist/main.js"
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
        dotnet build -c Release --nologo -v q sdk/conformance/settings/csharp >/dev/null
        FIXTURE=$SETTINGS compare csharp-settings dotnet sdk/conformance/settings/csharp/bin/Release/net10.0/Prefs.dll
        FIXTURE=$SETTINGS_FORM compare csharp-settings-form dotnet sdk/conformance/settings/csharp/bin/Release/net10.0/Prefs.dll --form
        query csharp-query 'Passed: +1,' dotnet test --nologo -v q sdk/dotnet/Celobus.Sdk.Tests \
          --filter 'FullyQualifiedName~QueryLinqTests.TheConformanceQueryIsTheSharedTree&FullyQualifiedName!~Analytic'
        query csharp-analytic 'Passed: +1,' dotnet test --nologo -v q sdk/dotnet/Celobus.Sdk.Tests \
          --filter FullyQualifiedName~QueryLinqTests.TheConformanceQueryIsTheSharedTreeOnTheAnalyticEngine
        query csharp-view-query 'Passed: +1,' dotnet test --nologo -v q sdk/dotnet/Celobus.Sdk.Tests \
          --filter FullyQualifiedName~ViewTests.TheConformanceViewQueryIsTheSharedTreeOnTheAnalyticEngine
        query csharp-decimal-query 'Passed: +1,' dotnet test --nologo -v q sdk/dotnet/Celobus.Sdk.Tests \
          --filter FullyQualifiedName~ViewTests.TheConformanceDecimalQueryIsTheSharedTreeOnTheAnalyticEngine
        lake csharp-lake dotnet "$PWD/sdk/conformance/jobs/csharp/bin/Release/net10.0/Jobsdemo.dll"
      else
        skip "csharp: dotnet is not installed"
      fi
      ;;
    *) echo "unknown language $lang (rust, ts, csharp)" >&2; exit 1 ;;
  esac
done
exit $fail
