#!/usr/bin/env bash
set -euo pipefail

# Compose a supergraph schema for the federation example.
# Requires: Apollo Rover CLI (`npm i -g @apollo/rover` or see https://www.apollographql.com/docs/rover/getting-started/)

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
CONFIG="${SCRIPT_DIR}/supergraph.yaml"
OUTPUT="${SCRIPT_DIR}/supergraph.graphql"

if ! command -v rover >/dev/null 2>&1; then
  echo "rover is required to compose the supergraph (see https://www.apollographql.com/docs/rover/getting-started/)" >&2
  exit 1
fi

# Accept the ELv2 license so rover can run non-interactively.
export APOLLO_ELV2_LICENSE=accept

echo "Composing supergraph with config: ${CONFIG}"
rover supergraph compose --config "${CONFIG}" > "${OUTPUT}"
echo "Wrote supergraph to ${OUTPUT}"
