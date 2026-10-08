#!/bin/sh
set -eu

archive=$1
name=$2
version=$3
root=$(mktemp -d /tmp/litradar-release.XXXXXX)
tar -xzf "/release-assets/$archive" -C "$root"
cd "$root/$name"
test ! -e web
test "$(./run.sh --version)" = "litradar $version"
./obscura --version
pdftotext -v
isolated=$(mktemp -d /tmp/litradar-executable.XXXXXX)
cp litradar "$isolated/litradar"
mkdir -p "$isolated/assets" "$isolated/data/index"
cp -R assets/meta "$isolated/assets/meta"
cp /smoke-fixture.sqlite "$isolated/data/index/smoke.sqlite"
cd "$isolated"
./litradar admin index optimize-storage --confirm-index-maintenance --project-root "$isolated"
printf 'SyntheticArchivePassword!2026\n' | ./litradar admin bootstrap --username static_smoke --password-stdin --project-root "$isolated"
head -c 32 /dev/urandom > secret.key
./litradar serve --host 127.0.0.1 --port 8000 --secret-key-file secret.key > service.log 2>&1 &
service=$!
trap 'kill "$service" 2>/dev/null || true; wait "$service" || true; cat service.log' EXIT
attempt=0
until curl --fail --silent http://127.0.0.1:8000/health/ready >/dev/null; do
    kill -0 "$service"
    attempt=$((attempt + 1))
    test "$attempt" -lt 60
    sleep 1
done
curl --fail --silent --dump-header /tmp/litradar-embedded-headers http://127.0.0.1:8000/ > /tmp/litradar-embedded-home
grep -i 'content-security-policy:.*sha256-' /tmp/litradar-embedded-headers >/dev/null
asset=$(sed -n 's/.*src="\(\/_next\/static\/[^" ]*\.js\)".*/\1/p' /tmp/litradar-embedded-home | head -n 1)
test -n "$asset"
curl --fail --silent "http://127.0.0.1:8000$asset" >/dev/null
stylesheet=$(sed -n 's/.*href="\(\/_next\/static\/[^" ]*\.css\)".*/\1/p' /tmp/litradar-embedded-home | head -n 1)
test -n "$stylesheet"
curl --fail --silent "http://127.0.0.1:8000$stylesheet" >/dev/null
curl --fail --silent http://127.0.0.1:8000/login >/dev/null
test "$(curl --silent --output /dev/null --write-out '%{http_code}' http://127.0.0.1:8000/missing-embedded-page)" = 404
test ! -e web
curl --fail --silent http://127.0.0.1:8000/openapi.json >/dev/null
test -f data/meta/chinese_journals.csv
test ! -e libsimple.so
curl --fail --silent --cookie-jar cookies -H 'Content-Type: application/json' --data '{"username":"static_smoke","password":"SyntheticArchivePassword!2026"}' http://127.0.0.1:8000/api/auth/login >/dev/null
curl --fail --silent --cookie cookies --get --data-urlencode db=smoke.sqlite --data-urlencode q=科技金融 http://127.0.0.1:8000/api/articles > search.json
grep '科技金融' search.json >/dev/null
curl --fail --silent --cookie cookies --get --data-urlencode db=smoke.sqlite --data-urlencode q=kejijinrong http://127.0.0.1:8000/api/articles > empty.json
grep '"items":\[\]' empty.json >/dev/null
kill -TERM "$service"
wait "$service"
trap - EXIT
