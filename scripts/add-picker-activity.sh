#!/usr/bin/env bash
# Add PhotoCraft's Activity subclass to the APK `cargo apk build` produced.
#
#   usage: add-picker-activity.sh [apk]
#
# Why this is a separate step: `cargo apk build` packages a native library and nothing else —
# it has no Java support at all, and the Activity in the manifest
# (`ai.storyteller.photocraft.PickerActivity`, see Cargo.toml) has to be a compiled class or
# the app will not start. This compiles the one class in apps/photocraft-android/java with
# `javac` + `d8`, drops the resulting `classes.dex` into the APK with `aapt add`, then
# re-aligns and re-signs — the same three steps cargo-apk runs, in the same order.
#
# Environment (see scripts/android-env.sh):
#   * $ANDROID_HOME / $ANDROID_SDK_ROOT — for build-tools and platforms/android-35
#   * $JAVA_HOME                      — javac
# and the keystore this app signs with, passed in through the usual cargo-apk variables:
#   CARGO_APK_RELEASE_KEYSTORE, CARGO_APK_RELEASE_KEYSTORE_PASSWORD
set -o pipefail
crate_dir="$(cd "$(dirname "$0")/.." && pwd)/apps/photocraft-android"
apk="${1:-target/release/apk/photocraft.apk}"

# javac, d8 and aapt are native programs, so they are handed native paths: under MSYS (git
# bash on Windows) that means converting, and elsewhere the path is already native.
native() { command -v cygpath >/dev/null 2>&1 && cygpath -w "$1" || printf '%s' "$1"; }

# build-tools ships `d8` on Linux and `d8.bat` on Windows, and both spellings have to work.
tool() { for ext in "" .bat .exe; do [ -f "$1$ext" ] && { printf '%s' "$1$ext"; return; }; done; printf '%s' "$1"; }

sdk="${ANDROID_SDK_ROOT:-${ANDROID_HOME:?set ANDROID_SDK_ROOT or ANDROID_HOME}}"
build_tools="$(ls -d "$sdk"/build-tools/*/ | sort -V | tail -1)"
platform="$(ls -d "$sdk"/platforms/*/ | sort -V | tail -1)"
android_jar="${platform}android.jar"
javac="${JAVA_HOME:+$JAVA_HOME/bin/javac}"
[ -x "$javac" ] || javac="$(command -v javac)" || { echo "javac not found; set JAVA_HOME"; exit 1; }
d8="$(tool "$build_tools/d8")"
aapt="$(tool "$build_tools/aapt")"
zipalign="$(tool "$build_tools/zipalign")"
apksigner="$(tool "$build_tools/apksigner")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
classes="$work/classes"
dex="$work/dex"
mkdir -p "$classes" "$dex"

echo "=== compiling $(basename "$crate_dir")/java ==="
find "$crate_dir/java" -name '*.java' | while read -r f; do native "$f"; done > "$work/sources.txt"
[ -s "$work/sources.txt" ] || { echo "no Java sources under $crate_dir/java"; exit 1; }
"$(native "$javac")" --release 17 -nowarn -classpath "$(native "$android_jar")" \
    -d "$(native "$classes")" "@$(native "$work/sources.txt")" || exit 1

# One class in, one classes.dex out. --lib is the framework jar desugaring needs.
find "$classes" -name '*.class' | while read -r f; do native "$f"; done > "$work/classes.txt"
"$(native "$d8")" --release --min-api 26 --lib "$(native "$android_jar")" \
    --output "$(native "$dex")" "@$(native "$work/classes.txt")" || exit 1
[ -f "$dex/classes.dex" ] || { echo "d8 produced no classes.dex"; exit 1; }

echo "=== adding classes.dex ($(wc -c < "$dex/classes.dex") bytes) to $(basename "$apk") ==="
# aapt appends to the archive it is given and the new file lands at the root, where the
# platform's class loader looks for it. Done on a copy, so a failed run leaves the signed APK
# alone.
staged="$work/$(basename "$apk")"
cp "$apk" "$staged"
(cd "$dex" && "$(native "$aapt")" add "$(native "$staged")" classes.dex) || exit 1

# Adding an entry moves what follows it, so the alignment cargo-apk did has to be redone — and
# with it the signature.
"$(native "$zipalign")" -f 4 "$(native "$staged")" "$(native "$apk")" || exit 1
"$(native "$apksigner")" sign \
    --ks "$(native "${CARGO_APK_RELEASE_KEYSTORE:?set CARGO_APK_RELEASE_KEYSTORE}")" \
    --ks-pass "pass:${CARGO_APK_RELEASE_KEYSTORE_PASSWORD:?set CARGO_APK_RELEASE_KEYSTORE_PASSWORD}" \
    "$(native "$apk")" || exit 1

echo "=== $(basename "$apk") ==="
"$(native "$aapt")" list "$(native "$apk")" | grep -x 'classes.dex' || {
    echo "classes.dex is missing from the APK; the app will not start"
    exit 1
}