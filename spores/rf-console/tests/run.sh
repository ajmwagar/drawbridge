#!/bin/sh
# Run the pure RF Deck tests. No browser, host, or network.
set -eu
cd "$(dirname "$0")/.."

SOIL_ROOT=${SOIL_ROOT:-$HOME/src/ajmwagar/soil}
SOIL=${SOIL:-$HOME/.local/bin/soil}

if [ ! -f "$SOIL_ROOT/core/prelude.soil" ]; then
  for candidate in "$HOME/src/soil-lang" "$HOME/src/soil"; do
    if [ -f "$candidate/core/prelude.soil" ]; then SOIL_ROOT=$candidate; break; fi
  done
fi

exec "$SOIL" run \
  --prelude "$SOIL_ROOT/core/prelude.soil" \
  "$SOIL_ROOT/core/ui.soil" \
  source/data.soil \
  source/bandmap.soil \
  source/ui.soil \
  tests/render.soil \
  --call test/all
