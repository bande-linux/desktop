#!/bin/bash
# Installs Debian 13's own font packages, the ones the render goldens are made with, on the
# Ubuntu runner image, whose font releases differ. Downloads are checked and cached.
set -euo pipefail

cache=${UNBOUND_FONT_CACHE:-/cache/debian-fonts}
mirror=https://deb.debian.org/debian
packages=(
    "f/fonts-dejavu/fonts-dejavu-mono_2.37-8_all.deb 3003e98a5debfdeadc7040a7f715fe9fe6fb67f68deacf6049b54e30f07fc014"
    "f/fonts-dejavu/fonts-dejavu-core_2.37-8_all.deb 86635b3d25b3655fc11cb3ecc3af59f0bf19643b02b94f2de48bd10253cdba12"
    "f/fonts-inter/fonts-inter_4.1+ds-1_all.deb a4dbb3dc2f8b012a881cd2be5b39e14b779a69dc9c1688eb15a8ea13f33fcb96"
    "f/fonts-manrope/fonts-manrope_4.504+git20201211-4_all.deb e1f2b17c5b39333ed2232075e54666634c2edc0ac53f58214d74926115589884"
    "f/fonts-jetbrains-mono/fonts-jetbrains-mono_2.304+ds-5_all.deb a3b42e1de29a93d8e78ae14ea6898a4534af5838ce946234ec3d8c7ab8ac271d"
    "f/fonts-noto-cjk/fonts-noto-cjk_20240730+repack1-1_all.deb f5dc28a754e17327d99f0a612134d92c8dd6187314ae967cb77f25df60860139"
    "f/fonts-noto-color-emoji/fonts-noto-color-emoji_2.051-0+deb13u1_all.deb 03141cd51c0e9ff8ad858fcf2d8234f8a00ae4d7a0872df43a71519359c54e8c"
)

"$(dirname "$0")/apt-install.sh" curl ca-certificates fontconfig
mkdir -p "$cache"
debs=()
for entry in "${packages[@]}"; do
    read -r pool sha256 <<< "$entry"
    deb="$cache/$(basename "$pool")"
    if ! echo "$sha256  $deb" | sha256sum --check --status 2>/dev/null; then
        curl -fsSL --retry 3 -o "$deb.part" "$mirror/pool/main/$pool"
        echo "$sha256  $deb.part" | sha256sum --check --quiet
        mv "$deb.part" "$deb"
    fi
    debs+=("$deb")
done
sudo dpkg --install "${debs[@]}"
fc-cache --force
