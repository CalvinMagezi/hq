#!/bin/sh
# Makes iroh 1.3 buildable for iOS: copies netdev and netwatch from the cargo registry into
# vendor/, applies the two small patches in patches/, and points cargo at them through
# .cargo/config.toml. Android and macOS need none of this. Run it once before building.
set -eu
cd "$(dirname "$0")"
cargo fetch --quiet
for crate in netdev-0.46.3 netwatch-0.19.3; do
  name="${crate%-*}"
  rm -rf "vendor/$name"
  mkdir -p vendor
  cp -R "$(ls -d "$HOME"/.cargo/registry/src/*/"$crate" | head -n 1)" "vendor/$name"
  patch -p1 -d "vendor/$name" < "patches/$name-ios.patch"
done
mkdir -p .cargo
cat > .cargo/config.toml <<'CFG'
[patch.crates-io]
netdev = { path = "vendor/netdev" }
netwatch = { path = "vendor/netwatch" }
CFG
echo "patched: vendor/netdev vendor/netwatch (see patches/)"
