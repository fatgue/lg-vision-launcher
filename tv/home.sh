#!/bin/sh
# Vision as Home on webOS 5/6 (run as root).
#
#   home.sh enable    make Vision the Home launcher (Home key opens Vision)
#   home.sh disable   restore LG's Home
#   home.sh status    show the current state
#   home.sh refresh   re-apply after a Vision update (the mount still holds the old binary)
#   (no argument)     boot hook: re-applies the mount shortly after a cold boot
#
# How: Vision's executable is bind-mounted over LG's Home executable. LG's Home manifest and
# LS2 role stay untouched; SAM starts Vision in its place and Vision registers as Home itself.
# Nothing on the read-only system partition changes; a mount does not survive a reboot, so the
# webosbrew boot hook re-applies it while Vision is enabled.

APP_ID=com.vision.launcher
HOME_ID=com.webos.app.home
EXE=/usr/bin/$HOME_ID
APPDIR=/media/developer/apps/usr/palm/applications/$APP_ID
STATE=/var/lib/vision-home
HOOK=/var/lib/webosbrew/init.d/64-vision-home
MARKER=/var/run/vision-home

say() { echo "Vision Home: $*"; }

luna() { luna-send -n 1 -w 4000 "luna://com.webos.applicationManager/$1" "$2" </dev/null >/dev/null 2>&1; }

mounted() { grep -q " $EXE " /proc/mounts; }

# Ends every process running LG's Home executable (the stock Home or Vision as Home).
stop_home() {
  for dir in /proc/[0-9]*; do
    [ "$( (tr '\0' '\n' < "$dir/cmdline") 2>/dev/null | head -n 1)" = "$EXE" ] && kill "${dir#/proc/}" 2>/dev/null
  done
  sleep 1
}

apply() {
  [ -f "$STATE/enabled" ] || return 0
  if [ ! -x "$APPDIR/vision" ]; then
    say "Vision is not installed; leaving LG Home in place"
    return 1
  fi
  if ! mounted; then
    mount --bind "$APPDIR/vision" "$EXE" || return 1
    # A stock Home started before the mount keeps running the old file: end it, so the
    # next Home key starts Vision.
    [ "$1" = boot ] && stop_home
  fi
  echo "$APP_ID" > "$MARKER"
}

unmount() {
  mounted || return 0
  umount "$EXE" 2>/dev/null || umount -l "$EXE"
}

case "${1:-boot}" in
  enable)
    [ "$(id -u)" = 0 ] || { say "run as root"; exit 1; }
    grep -q 'release [56]\.' /etc/starfish-release 2>/dev/null || { say "only webOS 5/6 is supported"; exit 1; }
    grep -q '"nativeLifeCycleInterfaceVersion": *2' /usr/palm/applications/$HOME_ID/appinfo.json \
      || { say "unexpected LG Home manifest"; exit 1; }
    [ -x "$APPDIR/vision" ] || { say "install Vision first"; exit 1; }
    mkdir -p "$STATE"
    cp "$0" "$STATE/home.sh.new" && chmod 755 "$STATE/home.sh.new" && mv "$STATE/home.sh.new" "$STATE/home.sh"
    touch "$STATE/enabled"
    ln -sf "$STATE/home.sh" "$HOOK"
    apply || { rm -f "$STATE/enabled" "$HOOK"; say "could not mount"; exit 1; }
    stop_home
    luna launch "{\"id\":\"$HOME_ID\"}"
    say "enabled. Restore LG Home with: sh $STATE/home.sh disable"
    ;;
  disable)
    rm -f "$STATE/enabled" "$HOOK" "$MARKER"
    unmount
    stop_home
    luna launch "{\"id\":\"$HOME_ID\"}"
    say "LG Home restored"
    ;;
  refresh)
    [ -f "$STATE/enabled" ] || exit 0
    # Run from an updated package: keep the boot hook's copy current.
    if [ "$0" != "$STATE/home.sh" ]; then
      cp "$0" "$STATE/home.sh.new" && chmod 755 "$STATE/home.sh.new" && mv "$STATE/home.sh.new" "$STATE/home.sh"
    fi
    unmount
    apply && stop_home && luna launch "{\"id\":\"$HOME_ID\"}"
    ;;
  status)
    if [ -f "$STATE/enabled" ]; then echo "enabled: yes"; else echo "enabled: no"; fi
    if mounted; then echo "mounted: yes"; else echo "mounted: no"; fi
    if [ -L "$HOOK" ]; then echo "boot hook: yes"; else echo "boot hook: no"; fi
    ;;
  apply)
    apply "$2"
    ;;
  boot)
    # From run-parts at boot: in the background, so a slow mount can never trip the
    # webosbrew failsafe.
    ( sleep 5; sh "$STATE/home.sh" apply boot ) </dev/null >/dev/null 2>&1 &
    ;;
  *)
    say "usage: home.sh enable|disable|status|refresh"
    exit 1
    ;;
esac
