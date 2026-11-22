#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."

actionlint_version=1.7.12
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
unset VK_LOADER_DRIVERS_DISABLE VK_LOADER_DRIVERS_SELECT VK_ICD_FILENAMES VK_ADD_DRIVER_FILES
export VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json
export CARGO_BUILD_RUSTFLAGS="${CARGO_BUILD_RUSTFLAGS:-} --remap-path-prefix=$PWD=/build --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo"
export CFLAGS="${CFLAGS:-} -ffile-prefix-map=$PWD=/build -ffile-prefix-map=${CARGO_HOME:-$HOME/.cargo}=/cargo"

setup=0
[ "${1:-}" != --setup ] || { setup=1; shift; }
[ $# -gt 0 ] || { echo "usage: verify.sh [--setup] lint|test|debs|gate..." >&2; exit 2; }
stages=()
for stage; do
    case $stage in
        gate) stages+=(lint test debs) ;;
        lint | test | debs) stages+=("$stage") ;;
        *) echo "verify.sh: no stage $stage" >&2; exit 2 ;;
    esac
done

packages() {
    case $1 in
        lint) echo curl ca-certificates shellcheck python3 jq rustfmt rust-clippy distro-info-data dpkg-dev ;;
        test) echo dpkg-dev xandikos ;;
        debs) echo build-essential dpkg-dev devscripts git jq lintian xz-utils distro-info-data ;;
    esac
}

actionlint() {
    local architecture sum cache tarball
    architecture=$(dpkg --print-architecture)
    case $architecture in
        amd64) sum=8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8 ;;
        arm64) sum=325e971b6ba9bfa504672e29be93c24981eeb1c07576d730e9f7c8805afff0c6 ;;
        *) echo "verify.sh: no actionlint for $architecture" >&2; exit 1 ;;
    esac
    cache=${XDG_CACHE_HOME:-$HOME/.cache}/bande-ci/actionlint-$actionlint_version
    if [ ! -x "$cache/actionlint" ]; then
        tarball=$(mktemp)
        curl -fsSL --retry 3 -o "$tarball" \
            "https://github.com/rhysd/actionlint/releases/download/v$actionlint_version/actionlint_${actionlint_version}_linux_$architecture.tar.gz"
        echo "$sum  $tarball" | sha256sum --check --quiet -
        mkdir -p "$cache"
        tar -xzf "$tarball" -C "$cache" actionlint
        rm -f "$tarball"
    fi
    "$cache/actionlint"
}

build_dep() {
    local as_root=()
    [ "$(id -u)" = 0 ] || as_root=(sudo)
    dpkg-checkbuilddeps -Ppkg.bande.vendor 2>/dev/null ||
        "${as_root[@]}" env DEBIAN_FRONTEND=noninteractive apt-get build-dep -y --no-install-recommends -Ppkg.bande.vendor ./
}

prepare() {
    local stage=$1 missing=() package
    mapfile -t wanted < <(packages "$stage" | xargs -n1)
    if [ $setup = 1 ]; then
        ci/apt-install.sh "${wanted[@]}"
        build_dep
        return
    fi
    for package in "${wanted[@]}"; do
        [ "$(dpkg-query -W -f='${db:Status-Status}' "$package" 2>/dev/null)" = installed ] || missing+=("$package")
    done
    [ ${#missing[@]} -eq 0 ] ||
        { echo "verify.sh: the $stage stage needs ${missing[*]}; install them (ci/apt-install.sh does)" >&2; exit 1; }
    dpkg-checkbuilddeps -Ppkg.bande.vendor ||
        { echo "verify.sh: install the Build-Depends (verify.sh --setup does)" >&2; exit 1; }
}

run() {
    case $1 in
        lint)
            actionlint
            ci/deb/version-test.sh
            ci/release/lint.sh
            ;;
        test)
            read -ra test_args <<< "${VERIFY_TEST_ARGS:---workspace}"
            dbus-run-session \
                --config-file=crates/testing/test-bus/data/session.conf -- cargo test --locked "${test_args[@]}"
            ;;
        debs) debs ;;
    esac
}

debs() {
    local out=${CARGO_TARGET_DIR:-target}/debian version
    version=$(ci/deb/version.sh)
    rm -rf "$out"
    ci/deb/source.sh "$out/source"
    DEB_BUILD_OPTIONS="nocheck ${DEB_BUILD_OPTIONS:-}" \
        ci/deb/build.sh "$out/source/bande-desktop_$(ci/deb/version.sh --source).dsc" "$out" --build=binary
    ci/release/check-debs.sh "$out" "$version"
    lintian --fail-on error,warning "$out"/*.changes
}

for stage in "${stages[@]}"; do
    prepare "$stage"
done
[ $setup = 0 ] || exit 0
for stage in "${stages[@]}"; do
    echo "== $stage"
    start=$SECONDS
    run "$stage"
    echo "== $stage passed in $((SECONDS - start)) s"
done
