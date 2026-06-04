#!/usr/bin/env bash
# Activate the venv created in this folder and run the demo.
set -euo pipefail
cd "$(dirname "$0")"
source .venv/bin/activate
exec python leap_demo.py
