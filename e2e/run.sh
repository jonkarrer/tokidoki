#!/bin/zsh
# Looks at the app (macOS, debug build): each test is a script run inside the window's page
# with `t` = window.__tokidoki (app/src/lib/debug.ts).
#
#   e2e/run.sh            run every test in e2e/tests
#   e2e/run.sh first-run  run one
#
# Each test starts from an empty data folder, e2e/.out/<name>.data, and the keys in the
# environment (the Keychain is not read: TOKIDOKI_NO_KEYCHAIN). Output and pictures go to
# e2e/.out/. A test that calls a model runs on the provider's cheapest (TOKIDOKI_MODELS=cheap).
# A test's <name>.env is its own environment, one NAME=value a line; $HOME and the like are
# filled in.
setopt null_glob
E2E=${0:A:h}
ROOT=${E2E:h}
OUT=$E2E/.out
mkdir -p $OUT

(cd $ROOT && cargo build -q -p tokidoki-app) || exit 1
curl -s -o /dev/null localhost:1430 || { (cd $ROOT/app && bun run dev > $OUT/vite.log 2>&1 &); }
for i in {1..80}; do curl -s -o /dev/null localhost:1430 && break; sleep 0.25; done

# The pages in e2e/fixtures, for the browser test, on a port of their own.
bun -e 'Bun.serve({ port: 1431, fetch: (r) => new Response(Bun.file(process.argv[1] + new URL(r.url).pathname)) })' $E2E/fixtures &
fixtures=$!
trap "kill $fixtures 2>/dev/null" EXIT

tests=(${@:-$E2E/tests/*.js(:t:r)})
failed=0
for name in $tests; do
  echo "== $name"
  rm -rf $OUT/$name.data $OUT/$name-*.png
  suite_env=()
  [[ -f $E2E/tests/$name.env ]] && suite_env=(${(e)${(f)"$(<$E2E/tests/$name.env)"}})
  env TOKIDOKI_MODELS=cheap $suite_env TOKIDOKI_TEST=$E2E/tests/$name.js TOKIDOKI_SHOTS=$OUT \
    TOKIDOKI_DATA=$OUT/$name.data TOKIDOKI_NO_KEYCHAIN=1 \
    $ROOT/target/debug/tokidoki-app > $OUT/$name.log 2>&1 &
  pid=$!
  for i in {1..480}; do grep -q "TEST DONE" $OUT/$name.log && break; kill -0 $pid 2>/dev/null || break; sleep 0.25; done
  kill $pid 2>/dev/null; wait $pid 2>/dev/null
  grep -E "^\[test\]|panicked" $OUT/$name.log | grep -v "TEST DONE"
  grep -q "TEST DONE" $OUT/$name.log || { echo "[test] FAIL the test never finished"; failed=1; }
  grep -qE "^\[test\] FAIL|panicked" $OUT/$name.log && failed=1
done
exit $failed
