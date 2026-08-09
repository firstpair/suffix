#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
pandoc --standalone --to man CLI-GUIDE.md --output man/suffix.1
