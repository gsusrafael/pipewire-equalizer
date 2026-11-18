#!/bin/sh
# Install pipewire-equalizer for the current user.
#
# Everything goes into ~/.local, and the daemon runs as a systemd user
# service, so no root is needed.
#
# SPDX-License-Identifier: GPL-3.0-or-later

set -eu

SRC=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

usage() {
	cat <<EOF
Usage: install.sh [options]

Install pipewire-equalizer for the current user.

  --prefix DIR   installation prefix (default: ~/.local)
  --no-enable    do not enable or start the systemd user service
  --uninstall    remove the installation (keeps the presets)
  --purge        with --uninstall, remove ~/.config/pweq as well
  -h, --help     show this help
EOF
}

die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }
info() { printf '%s\n' "$*"; }

prefix=${PREFIX:-${XDG_DATA_HOME:-$HOME/.local}}
enable=1
uninstall=0
purge=0

while [ $# -gt 0 ]; do
	case "$1" in
		--prefix) prefix=$2; shift 2 ;;
		--prefix=*) prefix=${1#*=}; shift ;;
		--no-enable) enable=0; shift ;;
		--uninstall) uninstall=1; shift ;;
		--purge) purge=1; shift ;;
		-h|--help) usage; exit 0 ;;
		*) usage >&2; die "unknown option: $1" ;;
	esac
done

bindir=$prefix/bin
datadir=$prefix/share/pipewire-equalizer
unitdir=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user

if [ "$uninstall" = 1 ]; then
	rm -f "$bindir/pweq-daemon" "$bindir/pweq-gui"
	rm -f "$unitdir/pweq-daemon.service"
	rm -rf "$datadir/presets"
	rmdir "$datadir" 2>/dev/null || true
	if command -v systemctl >/dev/null && [ "$enable" = 1 ]; then
		systemctl --user disable --now pweq-daemon.service 2>/dev/null || true
	fi
	info "removed the installation"
	if [ "$purge" = 1 ]; then
		rm -rf "${XDG_CONFIG_HOME:-$HOME/.config}/pweq"
		info "removed the configuration"
	fi
	exit 0
fi

command -v cargo >/dev/null || die "cargo is required to build"
command -v systemctl >/dev/null || die "systemd is required to run the daemon"

info "building the release binaries"
(cd "$SRC" && cargo build --release) || die "the build failed"

install -d "$bindir" "$datadir/presets" "$unitdir"

install -m 0755 "$SRC/target/release/pweq-daemon" "$bindir/pweq-daemon"
install -m 0755 "$SRC/target/release/pweq-gui" "$bindir/pweq-gui"
install -m 0644 "$SRC"/presets/*.json "$datadir/presets/"

cat > "$unitdir/pweq-daemon.service" <<EOF
[Unit]
Description=PipeWire equalizer
Documentation=https://github.com/gsusrafael/pipewire-equalizer
After=pipewire.service wireplumber.service
BindsTo=pipewire.service

[Service]
Type=simple
ExecStart=$bindir/pweq-daemon
Restart=on-failure
RestartSec=2

[Install]
WantedBy=pipewire.service
EOF

# The presets shipped with the project are copied once, so the user can edit
# their own without losing the originals.
user_presets=${XDG_CONFIG_HOME:-$HOME/.config}/pweq/presets
if [ ! -d "$user_presets" ]; then
	install -d "$user_presets"
	install -m 0644 "$SRC"/presets/*.json "$user_presets/"
fi

if [ "$enable" = 1 ]; then
	systemctl --user daemon-reload
	systemctl --user enable --now pweq-daemon.service
	info "the daemon is running"
else
	info "the systemd service was not enabled"
fi

info "installed in $prefix"
info "run the interface with: $bindir/pweq-gui"
