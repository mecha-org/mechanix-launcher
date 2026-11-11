#!/bin/sh
# Set or remove a user's lock screen PIN, in the pam_pwdfile file the
# mechanix-pin-check stack reads (0640 root:<user>).
# Usage: sudo ./make-pin.sh <4 or 6 digits> [user] | --unset [user]
set -eu

PIN_FILE="${MECHANIX_PIN_FILE:-/etc/mechanix/pin.passwd}"
USER="${2:-${SUDO_USER:?name the user}}"

remove() {
    if [ -f "$PIN_FILE" ]; then
        sed -i "/^${USER}:/d" "$PIN_FILE"
    fi
}

if [ "${1:-}" = "--unset" ]; then
    remove
    echo "PIN removed for ${USER}"
    exit 0
fi

PIN="${1:?usage: make-pin.sh <4 or 6 digits> [user] | --unset [user]}"
case "$PIN" in
    [0-9][0-9][0-9][0-9] | [0-9][0-9][0-9][0-9][0-9][0-9]) ;;
    *) echo "error: the PIN is 4 or 6 digits" >&2; exit 1 ;;
esac

HASH="$(printf '%s\n' "$PIN" | openssl passwd -6 -stdin)"
install -d -m 0750 -o root -g "$USER" "$(dirname "$PIN_FILE")"
remove
printf '%s:%s\n' "$USER" "$HASH" >> "$PIN_FILE"
chown root:"$USER" "$PIN_FILE"
chmod 0640 "$PIN_FILE"
echo "PIN set for ${USER} in ${PIN_FILE}"
