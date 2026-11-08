#!/bin/sh
# Installs the latest Parcela release into ~/.local/bin, plus Caddy when no supported version is
# on PATH. Run with --help for options.
# Served at https://parcela.run/install.sh from github.com/lostcoords/parcela.
set -eu

# The version Parcela's live tests run against.
CADDY_VERSION="${CADDY_VERSION:-2.11.7}"
BIN="$HOME/.local/bin"

usage() {
  cat <<EOF
Usage: install.sh [--nightly]

Installs Parcela into ~/.local/bin, plus Caddy 2 when no supported version
is on PATH.

Options:
  --nightly    Install the newest build of main instead of the latest release
  -h, --help   Show this help

Environment:
  PARCELA_SKIP_CADDY=1       Install only Parcela
  CADDY_VERSION              Caddy version to install (default $CADDY_VERSION)
EOF
}

# digest ALGORITHM FILE prints the file's hex digest.
digest() {
  if command -v "sha$1sum" >/dev/null 2>&1; then
    "sha$1sum" "$2" | cut -d' ' -f1
  else
    shasum -a "$1" "$2" | cut -d' ' -f1
  fi
}

download() {
  curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2"
}

# install_parcela RELEASE_URL installs the build for this machine, checked against its checksum.
install_parcela() {
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) target=aarch64-apple-darwin ;;
    Darwin-x86_64) target=x86_64-apple-darwin ;;
    Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
    Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-gnu ;;
    *) echo "no Parcela build for $(uname -s) $(uname -m); see https://parcela.run/docs/" >&2; exit 1 ;;
  esac
  release=$1
  work=$(mktemp -d)
  # The nightly assets are replaced in place, so a download can pair a new tarball with an old
  # checksum. Try once more before giving up.
  for attempt in 1 2; do
    download "$release/parcela-$target.tar.gz" "$work/parcela.tar.gz"
    download "$release/parcela-$target.tar.gz.sha256" "$work/sha256"
    expected=$(cut -d' ' -f1 "$work/sha256")
    [ -n "$expected" ] && [ "$(digest 256 "$work/parcela.tar.gz")" = "$expected" ] && break
    if [ "$attempt" = 2 ]; then
      echo "parcela: checksum mismatch; nothing installed" >&2
      rm -rf "$work"
      exit 1
    fi
    echo "parcela: checksum mismatch; retrying" >&2
    sleep 10
  done
  tar -xzf "$work/parcela.tar.gz" -C "$work"
  mkdir -p "$BIN"
  install -m 755 "$work/parcela-$target/parcela" "$BIN/parcela"
  rm -rf "$work"
  echo "installed $("$BIN/parcela" --version) to $BIN/parcela"
}

# fetch NAME URL CHECKSUMS_URL ARCHIVE ALGORITHM downloads ARCHIVE, checks it against the
# release's checksum file and installs the NAME binary from it into $BIN.
fetch() {
  work=$(mktemp -d)
  download "$2/$4" "$work/$4"
  download "$3" "$work/checksums"
  expected=$(grep " $4\$" "$work/checksums" | cut -d' ' -f1)
  if [ -z "$expected" ] || [ "$(digest "$5" "$work/$4")" != "$expected" ]; then
    echo "$1: checksum mismatch for $4; nothing installed" >&2
    rm -rf "$work"
    return 1
  fi
  tar -xzf "$work/$4" -C "$work" "$1"
  mkdir -p "$BIN"
  install -m 755 "$work/$1" "$BIN/$1"
  rm -rf "$work"
  echo "installed $1 to $BIN/$1"
  wrote="$wrote ~/.local/bin/$1"
}

# supported BINARY succeeds when BINARY is a Caddy version Parcela runs, matching its own check.
supported() {
  case "$("$1" version 2>/dev/null)" in v2.*) return 0 ;; esac
  return 1
}

# install_caddy installs Caddy into $BIN unless a supported one is on PATH or already there. A found
# binary of another version stays put; the pinned one goes into $BIN beside it.
install_caddy() {
  found=$(command -v caddy 2>/dev/null || true)
  [ -z "$found" ] && [ -x "$BIN/caddy" ] && found="$BIN/caddy"
  [ -n "$found" ] && supported "$found" && return 0
  if [ -n "$found" ] && [ "$found" != "$BIN/caddy" ] && [ -x "$BIN/caddy" ] && supported "$BIN/caddy"; then
    echo "caddy: unsupported version at $found; using ~/.local/bin/caddy"
    installed="$installed caddy"
    return 0
  fi
  [ -n "$found" ] && echo "caddy: unsupported version at $found; installing Caddy $CADDY_VERSION into ~/.local/bin"
  release="https://github.com/caddyserver/caddy/releases/download/v$CADDY_VERSION"
  fetch caddy "$release" "$release/caddy_${CADDY_VERSION}_checksums.txt" \
    "caddy_${CADDY_VERSION}_${caddy_os}_${arch}.tar.gz" 512
  installed="$installed caddy"
}

# path_hint prints how to put $BIN on PATH, or ahead of another Caddy that shadows it.
path_hint() {
  case ":$PATH:" in
    *":$BIN:"*)
      shadowed=
      for name in $installed; do
        found=$(command -v "$name")
        case "$found $shadowed " in
          "$BIN/$name "* | *" ${found%/*} "*) ;;
          *) shadowed="$shadowed ${found%/*}" ;;
        esac
      done
      [ -z "$shadowed" ] && return 0
      echo
      echo "Put ~/.local/bin ahead of$shadowed on PATH:"
      ;;
    *)
      echo
      echo "Add ~/.local/bin to PATH:"
      ;;
  esac
  case "${SHELL##*/}" in
    fish) echo "  fish_add_path --move ~/.local/bin" ;;
    zsh) echo "  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc" ;;
    bash) echo "  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc" ;;
    *) echo "  export PATH=\"\$HOME/.local/bin:\$PATH\"" ;;
  esac
}

# uninstall_hint names every file this script wrote.
uninstall_hint() {
  echo
  echo "To uninstall: rm$wrote"
}

main() {
  nightly=
  for arg in "$@"; do
    case "$arg" in
      --nightly) nightly=1 ;;
      -h | --help) usage; exit 0 ;;
      *) echo "unknown option: $arg" >&2; usage >&2; exit 2 ;;
    esac
  done

  # Rerunning this script is how an install.sh install upgrades.
  previous=$("$BIN/parcela" --version 2>/dev/null || true)
  if [ -n "$nightly" ]; then
    install_parcela https://github.com/lostcoords/parcela/releases/download/nightly
  else
    install_parcela https://github.com/lostcoords/parcela/releases/latest/download
  fi
  wrote=" ~/.local/bin/parcela"
  installed=

  if [ "${PARCELA_SKIP_CADDY:-}" != 1 ]; then
    case "$(uname -s)-$(uname -m)" in
      Darwin-x86_64) caddy_os=mac arch=amd64 ;;
      Darwin-arm64) caddy_os=mac arch=arm64 ;;
      Linux-x86_64 | Linux-amd64) caddy_os=linux arch=amd64 ;;
      Linux-aarch64 | Linux-arm64) caddy_os=linux arch=arm64 ;;
      *) caddy_os= ;;
    esac
    if [ -n "$caddy_os" ]; then
      install_caddy
    else
      echo "Install Caddy 2 yourself; run parcela doctor to check."
    fi
  fi

  path_hint
  echo
  current=$("$BIN/parcela" --version)
  refreshed=
  if "$BIN/parcela" proxy refresh --help >/dev/null 2>&1; then
    "$BIN/parcela" proxy refresh >/dev/null
    refreshed=1
  fi
  if [ -n "$previous" ] && [ "$previous" != "$current" ]; then
    echo "Upgraded from ${previous#parcela }."
    if [ -z "$refreshed" ]; then
      echo "This older release requires parcela proxy restart to refresh a running proxy."
    fi
    echo "Application processes keep running. Changes: https://github.com/lostcoords/parcela/blob/main/CHANGELOG.md"
  elif [ -z "$previous" ]; then
    echo "Next, in your project: parcela init, then parcela up"
  fi
  uninstall_hint
}

main "$@"
