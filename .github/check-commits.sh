#!/bin/sh
set -eu

hook="$(dirname "$0")/../.githooks/commit-msg"
message=$(mktemp)
status=0
for commit in $(git rev-list --no-merges "$@"); do
  git log -1 --format=%B "$commit" > "$message"
  "$hook" "$message" || status=1
done
rm -f "$message"
exit $status
