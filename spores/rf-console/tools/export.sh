#!/bin/sh
# Render the RF Deck page from Soil source into web/index.html.
set -eu
cd "$(dirname "$0")/.."

SOIL_ROOT=${SOIL_ROOT:-$HOME/src/ajmwagar/soil}
SOIL=${SOIL:-$HOME/.local/bin/soil}

if [ ! -f "$SOIL_ROOT/core/prelude.soil" ]; then
  for candidate in "$HOME/src/soil-lang" "$HOME/src/soil"; do
    if [ -f "$candidate/core/prelude.soil" ]; then SOIL_ROOT=$candidate; break; fi
  done
fi

if [ ! -x "$SOIL" ]; then
  echo "export: missing soil runner at $SOIL" >&2
  exit 2
fi

"$SOIL" run \
  --prelude "$SOIL_ROOT/core/prelude.soil" \
  "$SOIL_ROOT/core/ui.soil" \
  source/data.soil \
  source/bandmap.soil \
  source/ui.soil \
  --call main > web/index.html
