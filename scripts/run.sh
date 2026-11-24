#!/bin/sh
# `swift run` alone does not build libNotchCatMediaRemoteHelper.dylib; a plain
# `swift build` does. Always build first, then launch the built executable.
cd "$(dirname "$0")/.." || exit 1
swift build && exec .build/debug/NotchCat
