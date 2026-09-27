#!/bin/sh
# Installs whk, the Webhooker CLI, on Linux and macOS.
#
#   curl -fsSL https://webhooker.eu/install.sh | sh
#   curl -fsSL https://webhooker.eu/install.sh | sh -s -- --version v0.1.1 --dir /usr/local/bin
#
# WHK_VERSION and WHK_INSTALL_DIR work in place of the flags.

set -eu

REPOSITORY="webhooker-eu/webhooker-cli"
BINARY_NAME="whk"

say() {
    printf '%s\n' "$*"
}

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<EOF
Install whk, the Webhooker CLI.

Usage: install.sh [--version vX.Y.Z] [--dir DIRECTORY]

Options:
  --version   Release to install (default: latest)
  --dir       Directory to put whk in (default: \$HOME/.local/bin)
  -h, --help  Show this help
EOF
}

detect_target() {
    operating_system=$(uname -s)
    machine=$(uname -m)

    case "$machine" in
        x86_64 | amd64) architecture="x86_64" ;;
        aarch64 | arm64) architecture="aarch64" ;;
        *) fail "unsupported CPU architecture: $machine" ;;
    esac

    case "$operating_system" in
        Linux) printf '%s-unknown-linux-musl' "$architecture" ;;
        Darwin)
            # A shell running under Rosetta reports x86_64 on Apple Silicon.
            if [ "$architecture" = "x86_64" ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = "1" ]; then
                architecture="aarch64"
            fi
            printf '%s-apple-darwin' "$architecture"
            ;;
        MINGW* | MSYS* | CYGWIN*)
            fail "on Windows, run in PowerShell: irm https://webhooker.eu/install.ps1 | iex"
            ;;
        *) fail "unsupported operating system: $operating_system" ;;
    esac
}

download() {
    source_url=$1
    destination_path=$2
    if command -v curl >/dev/null 2>&1; then
        curl --proto '=https' --tlsv1.2 -fsSL "$source_url" -o "$destination_path"
    elif command -v wget >/dev/null 2>&1; then
        wget --https-only -q "$source_url" -O "$destination_path"
    else
        fail "curl or wget is required"
    fi
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        fail "sha256sum or shasum is required to verify the download"
    fi
}

main() {
    requested_version=${WHK_VERSION:-latest}
    install_directory=${WHK_INSTALL_DIR:-"$HOME/.local/bin"}

    while [ $# -gt 0 ]; do
        case "$1" in
            --version)
                [ $# -ge 2 ] || fail "--version needs a value"
                requested_version=$2
                shift 2
                ;;
            --dir)
                [ $# -ge 2 ] || fail "--dir needs a value"
                install_directory=$2
                shift 2
                ;;
            -h | --help)
                usage
                exit 0
                ;;
            *) fail "unknown option: $1 (see --help)" ;;
        esac
    done

    target=$(detect_target)
    archive_name="$BINARY_NAME-$target.tar.gz"

    if [ "$requested_version" = "latest" ]; then
        release_url="https://github.com/$REPOSITORY/releases/latest/download"
    else
        case "$requested_version" in
            v*) ;;
            *) requested_version="v$requested_version" ;;
        esac
        release_url="https://github.com/$REPOSITORY/releases/download/$requested_version"
    fi

    temporary_directory=$(mktemp -d)
    trap 'rm -rf "$temporary_directory"' EXIT INT TERM

    say "Downloading $archive_name ($requested_version)"
    download "$release_url/$archive_name" "$temporary_directory/$archive_name" \
        || fail "download failed; check that release $requested_version exists"
    download "$release_url/$archive_name.sha256" "$temporary_directory/$archive_name.sha256" \
        || fail "checksum file is missing for release $requested_version"

    expected_checksum=$(cut -d ' ' -f 1 "$temporary_directory/$archive_name.sha256")
    actual_checksum=$(sha256_of "$temporary_directory/$archive_name")
    [ "$expected_checksum" = "$actual_checksum" ] \
        || fail "checksum mismatch for $archive_name (expected $expected_checksum, got $actual_checksum)"

    tar -xzf "$temporary_directory/$archive_name" -C "$temporary_directory"
    mkdir -p "$install_directory"
    mv -f "$temporary_directory/$BINARY_NAME" "$install_directory/$BINARY_NAME"
    chmod 755 "$install_directory/$BINARY_NAME"

    say "Installed $("$install_directory/$BINARY_NAME" --version) to $install_directory/$BINARY_NAME"

    case ":$PATH:" in
        *":$install_directory:"*) ;;
        *)
            say ""
            say "$install_directory is not on your PATH. Add it to your shell profile:"
            say "  export PATH=\"$install_directory:\$PATH\""
            ;;
    esac

    say ""
    say "Next: create an API key at https://app.webhooker.eu and run: whk login"
}

main "$@"
