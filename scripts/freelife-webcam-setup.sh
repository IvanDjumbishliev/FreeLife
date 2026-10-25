#!/usr/bin/env bash
# Create the virtual camera that FreeLife writes a phone's camera into.
#
# Loads the v4l2loopback kernel module with one device at /dev/video10,
# labelled "FreeLife Camera". Needs root. Run it again after a reboot, or
# set it up at boot with the modules-load and modprobe.d files it prints.
#
#   sudo ./scripts/freelife-webcam-setup.sh
#   sudo ./scripts/freelife-webcam-setup.sh --unload

set -euo pipefail

VIDEO_NR=10
LABEL="FreeLife Camera"

if [[ $EUID -ne 0 ]]; then
  echo "This script loads a kernel module and must run as root (try sudo)." >&2
  exit 1
fi

if [[ "${1:-}" == "--unload" ]]; then
  modprobe -r v4l2loopback
  echo "FreeLife Camera removed."
  exit 0
fi

if ! modinfo v4l2loopback >/dev/null 2>&1; then
  cat >&2 <<MSG
The v4l2loopback module isn't installed.

  Debian/Ubuntu:  sudo apt install v4l2loopback-dkms
  Fedora:         sudo dnf install v4l2loopback
  Arch:           sudo pacman -S v4l2loopback-dkms

Then run this script again.
MSG
  exit 1
fi

if [[ -e /dev/video$VIDEO_NR ]] && grep -q "$LABEL" /sys/class/video4linux/video$VIDEO_NR/name 2>/dev/null; then
  echo "$LABEL is already at /dev/video$VIDEO_NR."
  exit 0
fi

modprobe v4l2loopback devices=1 video_nr=$VIDEO_NR card_label="$LABEL" exclusive_caps=1

echo "$LABEL is ready at /dev/video$VIDEO_NR."
echo
echo "To load it at every boot, run once as root:"
echo "  echo 'options v4l2loopback devices=1 video_nr=$VIDEO_NR card_label=\"$LABEL\" exclusive_caps=1' > /etc/modprobe.d/freelife-camera.conf"
echo "  echo v4l2loopback > /etc/modules-load.d/freelife-camera.conf"
