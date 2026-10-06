#!/bin/sh
# Export the exact Go build graph, toolchain and license inputs with the binary.
set -eu
mkdir -p /out/inventory/licenses
go version > /out/inventory/toolchain.txt
go env -json > /out/inventory/environment.json
"$CC" --version > /out/inventory/compiler.txt
go list -mod=readonly -m -json all > /out/inventory/modules.json
go version -m /out/litradar > /out/inventory/binary-modules.txt
cp go.mod go.sum /out/inventory/
sha256sum /out/litradar > /out/inventory/binary.sha256
go list -mod=readonly -m -f '{{if .Replace}}{{.Replace.Dir}}{{else}}{{.Dir}}{{end}}' all |
while IFS= read -r module_directory; do
    test -n "$module_directory" || continue
    test "$module_directory" != /app || continue
    find "$module_directory" -maxdepth 1 -type f \( -iname 'license*' -o -iname 'notice*' -o -iname 'copying*' \) \
        -exec cp --parents --target-directory=/out/inventory/licenses {} +
done
find /out/inventory/licenses -type d -exec chmod u+w {} +
cp /usr/local/go/LICENSE /out/inventory/licenses/Go-LICENSE
find third_party -type f -print0 | sort -z | xargs -0 sha256sum > /out/inventory/patched-sources.sha256

find cmd internal assets -type f -print0 | sort -z | xargs -0 sha256sum > /out/inventory/application-sources.sha256
