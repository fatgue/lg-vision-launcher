#!/bin/sh
# Downloads the runtime libraries Vision loads with dlopen (not committed):
#   third_party/sdl-webos/libSDL2-2.0.so.0   SDL-webOS for the TV (bundled into the IPK)
#   third_party/sdl-windows/SDL2.dll         SDL2 for the desktop preview
set -eu
cd "$(dirname "$0")/.."
SDL_WEBOS_TAG=release-2.30.12-webos.5
SDL_WINDOWS_TAG=release-2.32.10

mkdir -p third_party/sdl-webos third_party/sdl-windows
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

gh release download "$SDL_WEBOS_TAG" -R webosbrew/SDL-webOS -p 'SDL2-*-webos.tar.gz' -D "$tmp"
tar -xzf "$tmp"/SDL2-*-webos.tar.gz -C "$tmp" ./lib/libSDL2-2.0.so.0.3000.12 ./share/licenses/SDL2/LICENSE.txt
cp "$tmp/lib/libSDL2-2.0.so.0.3000.12" third_party/sdl-webos/libSDL2-2.0.so.0
cp "$tmp/share/licenses/SDL2/LICENSE.txt" third_party/sdl-webos/LICENSE-SDL2.txt

gh release download "$SDL_WINDOWS_TAG" -R libsdl-org/SDL -p 'SDL2-*-win32-x64.zip' -D "$tmp"
unzip -o -q "$tmp"/SDL2-*-win32-x64.zip SDL2.dll -d third_party/sdl-windows

echo "third_party ready"
