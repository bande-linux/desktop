#!/bin/bash
# prepare.sh <debian source dir> <version>: turns Debian's firefox-esr source into bande-browser:
# package identity, build options, branding, prefs, search config and our patches after Debian's.
# Exits 3 when one of our patches doesn't apply, leaving quilt at the failing patch, and 4 when we
# have no patches for this ESR.
set -euo pipefail
src=$(realpath "$1") version=$2
repo=$(dirname "$(dirname "$(realpath "$0")")")
cd "$src"
export QUILT_PATCHES=debian/patches QUILT_PC=.pc

debver=$(dpkg-parsechangelog -S Version)
major=${debver%%.*}
patches=$repo/patches/$major
[ -f "$patches/series" ] || { echo "prepare: no patches for ESR $major" >&2; exit 4; }

quilt push -a -q > /dev/null 2>&1 || [ "$(quilt next 2>/dev/null || true)" = "" ]
mkdir -p debian/patches/bande
while read -r patch; do
    [ -n "$patch" ] && [ "${patch#\#}" = "$patch" ] || continue
    mkdir -p "debian/patches/bande/$(dirname "$patch")"
    cp "$patches/$patch" "debian/patches/bande/$patch"
    echo "bande/$patch" >> debian/patches/series
done < "$patches/series"
if ! quilt push -a -q --fuzz=0 > /dev/null; then
    echo "prepare: $(quilt next) doesn't apply" >&2
    exit 3
fi

find . -type f -name '*.yaml' -not -path './third_party/*' -exec sed -i 's/follows_collection_enabled: false/follows_collection_enabled: true/' {} +

{
    echo "bande-browser ($version) unstable; urgency=medium"
    echo
    echo "  * Bande's Browser: Debian firefox-esr $debver with the patches of bande-linux/web."
    echo
    echo " -- Bande <noreply@bande-linux.github.io>  $(date -R)"
    echo
    cat debian/changelog
} > debian/changelog.new
mv debian/changelog.new debian/changelog

python3 - debian/browser.mozconfig.in debian/rules debian/browser.install.in debian/browser.links.in <<'PY'
import re, sys
mozconfig, rules, install, links = sys.argv[1:]
s = open(mozconfig).read()
s = s.replace('--enable-default-toolkit=cairo-gtk3-wayland\n', '--enable-default-toolkit=cairo-gtk3-wayland-only\n')
s = re.sub(r'ac_add_options --with-(google-location-service|google-safebrowsing|mozilla)-api-keyfile=.*\n', '', s)
s += '''ac_add_options --disable-crashreporter
ac_add_options --disable-debug-symbols
ac_add_options --disable-default-browser-agent
ac_add_options --enable-proxy-bypass-protection
export MOZ_TELEMETRY_REPORTING=
mk_add_options MOZ_CRASHREPORTER=0
mk_add_options MOZ_DATA_REPORTING=0
mk_add_options MOZ_SERVICES_HEALTHREPORT=0
mk_add_options MOZ_TELEMETRY_REPORTING=0
'''
open(mozconfig, 'w').write(s)
s = open(rules).read()
s = s.replace('OFFICIAL_BRANDING := browser/branding/official\n', 'OFFICIAL_BRANDING := browser/branding/official\nBRANDING := browser/branding/bande\n')
s = re.sub(r'\n\s*CRASH_REPORTER = 1', '\n\tNO_CRASH_REPORTER = 1', s)
s = s.replace('export MOZ_APP_REMOTINGNAME := $($(PRODUCT))', 'export MOZ_APP_REMOTINGNAME := org.bande.Browser')
open(rules, 'w').write(s)
s = open(install).read()
s = s.replace('debian/@browser@.desktop usr/share/applications\n', 'debian/org.bande.Browser.desktop usr/share/applications\ndebian/org.bande.Browser.metainfo.xml usr/share/metainfo\ndebian/org.bande.Browser.svg usr/share/icons/hicolor/scalable/apps\ndebian/org.bande.Browser.PictureInPicture.desktop usr/share/applications\ndebian/org.bande.Browser.Sharing.desktop usr/share/applications\ndebian/org.bande.Browser.PictureInPicture.svg usr/share/icons/hicolor/scalable/apps\ndebian/org.bande.Browser.Sharing.svg usr/share/icons/hicolor/scalable/apps\n')
s = s.replace('debian/@browser@-symbolic.svg usr/share/icons/hicolor/symbolic/apps\n', '')
s = s.replace('usr/lib/@browser@/pingsender\n', '')
s = s.replace('debian/vendor.js usr/share/@browser@/browser/defaults/preferences\n', 'debian/00-bande.js usr/share/@browser@/browser/defaults/preferences\n')
s = s.replace('%if browser != firefox\ndebian/firefox usr/bin\n%endif\n', '')
open(install, 'w').write(s)
PY
sed -i "s/^%define RAW_DESCRIPTION .*/%define RAW_DESCRIPTION Bande's web browser, based on Mozilla Firefox source code/" debian/control.in
sed -i 's/^Depends: ${shlibs:Depends},/Depends: bande-compositor (>= 0.1.8), ${shlibs:Depends},/' debian/control.in
rm -f debian/google.key debian/mls.key debian/browser.desktop.in debian/browser-symbolic.svg.in \
    debian/browser.preinst.in debian/browser.postrm.in debian/firefox.in
sed -i 's/^stamps\/dh_install:: debian\/noinstall debian\/$($(PRODUCT))-symbolic.svg/stamps\/dh_install:: debian\/noinstall/' debian/rules
cp "$repo/prefs/bande.js" debian/00-bande.js
for window in PictureInPicture:Picture-in-Picture Sharing:Sharing\ Indicator; do
    printf '[Desktop Entry]\nType=Application\nName=%s\nIcon=org.bande.Browser\nExec=bande-browser\nNoDisplay=true\n' "${window#*:}" > "debian/org.bande.Browser.${window%%:*}.desktop"
    cp "$repo/branding/org.bande.Browser.svg" "debian/org.bande.Browser.${window%%:*}.svg"
done
cp "$repo/branding/org.bande.Browser.desktop" "$repo/branding/org.bande.Browser.metainfo.xml" "$repo/branding/org.bande.Browser.svg" debian/

b=browser/branding/bande
cp -a browser/branding/unofficial "$b"
cp "$repo/branding/brand.ftl" "$repo/branding/brand.properties" "$b/locales/en-US/"
sed -i 's/^MOZ_APP_DISPLAYNAME=.*/MOZ_APP_DISPLAYNAME=Browser/; s/^MOZ_MACBUNDLE_ID=.*/MOZ_MACBUNDLE_ID=browser/' "$b/configure.sh"
sed -i '/app\.update\./d' "$b/pref/firefox-branding.js"
svg=$repo/branding/org.bande.Browser.svg
for size in 16 22 24 32 48 64 128 256; do rsvg-convert -w "$size" -h "$size" "$svg" -o "$b/default$size.png"; done
cp "$svg" "$b/content/about-logo.svg"
rsvg-convert -w 192 -h 192 "$svg" -o "$b/content/about-logo.png"
rsvg-convert -w 384 -h 384 "$svg" -o "$b/content/about-logo@2x.png"
cp "$b/content/about-logo.png" "$b/content/about-logo-private.png"
cp "$b/content/about-logo@2x.png" "$b/content/about-logo-private@2x.png"
rsvg-convert -w 210 -h 210 "$svg" -o "$b/content/about.png"
blank='<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>'
echo "$blank" > "$b/content/about-wordmark.svg"
echo "$blank" > "$b/content/firefox-wordmark.svg"

cp "$repo/search/search-config-v2.json" "$repo/search/search-config-icons.json" services/settings/dumps/main/
cp "$repo"/search/2c4b8834-030c-4097-a887-c7506689095c* services/settings/dumps/main/search-config-icons/

find browser/locales/en-US toolkit/locales/en-US dom/locales/en-US -type f \( -name '*.ftl' -o -name '*.properties' -o -name '*.dtd' \) \
    -exec sed -i -E '/^\s*(#|<!--)/! s/\bFirefox\b/Browser/g' {} +
cat "$repo/theme/bande.css" >> browser/themes/linux/browser.css
cp "$repo/l10n/languages" debian/bande-languages
sed -i 's/^L10N_LANGS = .*/L10N_LANGS = $(shell cat debian\/bande-languages)/' debian/upstream.mk
sed -i 's/^         @browser@ (>= ${source:Version}), @browser@ (<< ${source:Version}.1~)$/         @browser@ (= ${source:Version})\nEnhances: @browser@/' debian/l10n/browser-l10n.control.in
grep -q '^Enhances: @browser@$' debian/l10n/browser-l10n.control.in
for lang in $(cat "$repo/l10n/languages"); do
    find "l10n-$lang" -type f \( -name '*.ftl' -o -name '*.properties' -o -name '*.dtd' \) \
        -exec sed -i -E '/^\s*(#|<!--)/! s/\bFirefox\b/Browser/g' {} +
done
printf '\nmain-context-menu-bande-look-up =\n    .label = Slå upp ”{ $word }”\n' >> l10n-sv-SE/browser/browser/browserContext.ftl
python3 "$repo/scripts/fetch-tracking-lists.py" .
python3 "$repo/scripts/gut.py" . browser/components/asrouter browser/components/ipprotection \
    toolkit/components/ipprotection browser/components/genai browser/components/aiwindow \
    browser/components/reportbrokensite toolkit/components/reportbrokensite browser/components/messagepreview
cp "$repo"/theme/pages/* browser/base/content/
cp "$repo/l10n/en-US/bande.ftl" browser/locales/en-US/browser/
for lang in $(cat "$repo/l10n/languages"); do
    [ ! -f "$repo/l10n/$lang/bande.ftl" ] || cp "$repo/l10n/$lang/bande.ftl" "l10n-$lang/browser/browser/"
done
cat "$repo/theme/preferences.css" >> browser/themes/shared/preferences/preferences.css

find browser toolkit -type f \( -iname '*fox*.svg' -o -name 'kit-*.svg' -o -name 'kit.svg' \) -not -path '*/test*' \
    -exec sh -c 'echo "$0" > "$1"' "$blank" {} \;
cp "$svg" browser/themes/shared/sidebar/firefox.svg
cp "$svg" browser/themes/shared/icons/window-firefox.svg

debian/rules debian/control
grep -q '^Source: bande-browser$' debian/control
! grep -q 'debian/firefox usr/bin' debian/browser.install.in
