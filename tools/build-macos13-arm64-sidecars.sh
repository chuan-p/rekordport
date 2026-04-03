#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
OUT_DIR="${REPO_ROOT}/src-tauri/bin"
BUILD_ROOT="${BUILD_ROOT:-/tmp/rkb-macos13-sidecars}"
DEPLOYMENT_TARGET="${DEPLOYMENT_TARGET:-13.0}"
MAKE_JOBS="${MAKE_JOBS:-4}"

SQLCIPHER_VERSION="${SQLCIPHER_VERSION:-v4.13.0}"
FFMPEG_VERSION="${FFMPEG_VERSION:-8.1}"
LAME_VERSION="${LAME_VERSION:-3.100}"

mkdir -p "${OUT_DIR}" "${BUILD_ROOT}"

fetch_archive() {
  local url="$1"
  local output="$2"
  curl -L --fail --retry 3 --output "${output}" "${url}"
}

extract_archive() {
  local archive="$1"
  local destination="$2"
  rm -rf "${destination}"
  mkdir -p "${destination}"
  tar -xf "${archive}" -C "${destination}" --strip-components=1
}

echo "[sidecars] build root: ${BUILD_ROOT}"
echo "[sidecars] output dir: ${OUT_DIR}"
echo "[sidecars] deployment target: macOS ${DEPLOYMENT_TARGET}"

SQLCIPHER_ARCHIVE="${BUILD_ROOT}/sqlcipher-${SQLCIPHER_VERSION}.tar.gz"
SQLCIPHER_SRC="${BUILD_ROOT}/sqlcipher-src"
SQLCIPHER_BUILD="${BUILD_ROOT}/sqlcipher-build"
fetch_archive "https://github.com/sqlcipher/sqlcipher/archive/refs/tags/${SQLCIPHER_VERSION}.tar.gz" "${SQLCIPHER_ARCHIVE}"
extract_archive "${SQLCIPHER_ARCHIVE}" "${SQLCIPHER_SRC}"
rm -rf "${SQLCIPHER_BUILD}"
cp -R "${SQLCIPHER_SRC}" "${SQLCIPHER_BUILD}"
(
  cd "${SQLCIPHER_BUILD}"
  env \
    CC=clang \
    CFLAGS="-Os -arch arm64 -mmacosx-version-min=${DEPLOYMENT_TARGET} -DSQLITE_HAS_CODEC -DSQLITE_ENABLE_JSON1 -DSQLITE_ENABLE_FTS3 -DSQLITE_ENABLE_FTS3_PARENTHESIS -DSQLITE_ENABLE_FTS5 -DSQLITE_ENABLE_COLUMN_METADATA -DSQLITE_EXTRA_INIT=sqlcipher_extra_init -DSQLITE_EXTRA_SHUTDOWN=sqlcipher_extra_shutdown -DSQLCIPHER_CRYPTO_CC" \
    LDFLAGS="-arch arm64 -mmacosx-version-min=${DEPLOYMENT_TARGET} -framework Security -framework CoreFoundation" \
    ./configure --disable-tcl --enable-load-extension --with-tempstore=yes
  make -j"${MAKE_JOBS}" sqlite3
)

LAME_ARCHIVE="${BUILD_ROOT}/lame-${LAME_VERSION}.tar.gz"
LAME_SRC="${BUILD_ROOT}/lame-src"
LAME_BUILD="${BUILD_ROOT}/lame-build"
LAME_INSTALL="${BUILD_ROOT}/lame-install"
fetch_archive "https://downloads.sourceforge.net/project/lame/lame/${LAME_VERSION}/lame-${LAME_VERSION}.tar.gz" "${LAME_ARCHIVE}"
extract_archive "${LAME_ARCHIVE}" "${LAME_SRC}"
rm -rf "${LAME_BUILD}" "${LAME_INSTALL}"
cp -R "${LAME_SRC}" "${LAME_BUILD}"
(
  cd "${LAME_BUILD}"
  env \
    CC=clang \
    CFLAGS="-Os -arch arm64 -mmacosx-version-min=${DEPLOYMENT_TARGET}" \
    LDFLAGS="-arch arm64 -mmacosx-version-min=${DEPLOYMENT_TARGET}" \
    ./configure --prefix="${LAME_INSTALL}" --disable-shared --enable-static --host=arm-apple-darwin
  make -j"${MAKE_JOBS}"
  make install
)

FFMPEG_ARCHIVE="${BUILD_ROOT}/ffmpeg-${FFMPEG_VERSION}.tar.xz"
FFMPEG_SRC="${BUILD_ROOT}/ffmpeg-src"
FFMPEG_BUILD="${BUILD_ROOT}/ffmpeg-build"
fetch_archive "https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz" "${FFMPEG_ARCHIVE}"
extract_archive "${FFMPEG_ARCHIVE}" "${FFMPEG_SRC}"
rm -rf "${FFMPEG_BUILD}"
cp -R "${FFMPEG_SRC}" "${FFMPEG_BUILD}"
(
  cd "${FFMPEG_BUILD}"
  env \
    CC=clang \
    CFLAGS="-Os -arch arm64 -mmacosx-version-min=${DEPLOYMENT_TARGET}" \
    LDFLAGS="-arch arm64 -mmacosx-version-min=${DEPLOYMENT_TARGET}" \
    ./configure \
      --arch=arm64 \
      --target-os=darwin \
      --cc=clang \
      --enable-static \
      --disable-shared \
      --disable-debug \
      --disable-doc \
      --disable-ffplay \
      --disable-autodetect \
      --disable-indevs \
      --disable-outdevs \
      --disable-network \
      --disable-appkit \
      --disable-avfoundation \
      --disable-coreimage \
      --disable-metal \
      --disable-securetransport \
      --disable-videotoolbox \
      --disable-xlib \
      --enable-audiotoolbox \
      --enable-libmp3lame \
      --extra-cflags="-arch arm64 -mmacosx-version-min=${DEPLOYMENT_TARGET} -I${LAME_INSTALL}/include" \
      --extra-ldflags="-arch arm64 -mmacosx-version-min=${DEPLOYMENT_TARGET} -L${LAME_INSTALL}/lib" \
      --extra-libs="-lm"
  make -j"${MAKE_JOBS}" ffmpeg ffprobe
)

install -m 0755 "${SQLCIPHER_BUILD}/sqlite3" "${OUT_DIR}/sqlcipher-aarch64-apple-darwin"
install -m 0755 "${FFMPEG_BUILD}/ffmpeg" "${OUT_DIR}/ffmpeg-aarch64-apple-darwin"
install -m 0755 "${FFMPEG_BUILD}/ffprobe" "${OUT_DIR}/ffprobe-aarch64-apple-darwin"

echo "[sidecars] installed:"
echo "  ${OUT_DIR}/sqlcipher-aarch64-apple-darwin"
echo "  ${OUT_DIR}/ffmpeg-aarch64-apple-darwin"
echo "  ${OUT_DIR}/ffprobe-aarch64-apple-darwin"
