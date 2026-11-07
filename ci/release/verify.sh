#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."

cargo_deb_version=3.8.0
actionlint_version=1.7.12
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json
export CARGO_BUILD_RUSTFLAGS="${CARGO_BUILD_RUSTFLAGS:-} --remap-path-prefix=$PWD=/build --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/rustup"

setup=0
[ "${1:-}" != --setup ] || { setup=1; shift; }
[ $# -gt 0 ] || { echo "usage: verify.sh [--setup] lint|test|debs|all|gate..." >&2; exit 2; }
stages=()
for stage; do
    case $stage in
        gate) stages+=(lint test all debs) ;;
        lint | test | debs | all) stages+=("$stage") ;;
        *) echo "verify.sh: no stage $stage" >&2; exit 2 ;;
    esac
done

packages() {
    case $1 in
        lint) echo curl ca-certificates shellcheck ;;
        test) echo libvulkan1 mesa-vulkan-drivers dbus-daemon dbus-bin at-spi2-core libfontconfig1 \
            libwayland-client0 procps fonts-inter pipewire-bin ;;
        debs) echo dpkg-dev libvulkan1 mesa-vulkan-drivers dbus-daemon dbus-bin systemd dbus-user-session ;;
        all) echo build-essential debhelper dpkg-dev lintian ;;
    esac
    [ "$1" = all ] || grep -vx rustup ci/release/build-depends
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

cargo_deb() {
    cargo deb --version 2>/dev/null | grep -qw "$cargo_deb_version" ||
        cargo install cargo-deb --version "$cargo_deb_version" --locked
}

prepare() {
    local stage=$1 missing=() package
    mapfile -t wanted < <(packages "$stage" | xargs -n1)
    if [ $setup = 1 ]; then
        ci/apt-install.sh "${wanted[@]}"
        [ "$stage" = all ] || command -v rustup > /dev/null || ci/apt-install.sh rustup
        [ "$stage" = all ] || ci/release/toolchain.sh
        return
    fi
    for package in "${wanted[@]}"; do
        [ "$(dpkg-query -W -f='${db:Status-Status}' "$package" 2>/dev/null)" = installed ] || missing+=("$package")
    done
    [ "$stage" = all ] || command -v rustup > /dev/null || missing+=(rustup)
    [ ${#missing[@]} -eq 0 ] ||
        { echo "verify.sh: the $stage stage needs ${missing[*]}; install them (ci/apt-install.sh does)" >&2; exit 1; }
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
        debs) cargo_deb && ci/release/build-debs.sh && ci/release/check-debs.sh && ci/release/smoke.sh ;;
        all) ci/release/build-all.sh ;;
    esac
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
