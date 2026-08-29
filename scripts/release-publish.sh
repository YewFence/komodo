#!/usr/bin/env bash
set -euo pipefail

#USAGE arg "<tag>" help="Fixed release tag to create or update in place"
#USAGE flag "--title <title>" help="Release title" default="Komodo Rolling Build"
#USAGE flag "--notes <notes>" help="Release notes; empty generates commit and build timestamp"
#USAGE flag "--dry-run" help="Print the release plan without calling gh"

TAG="${usage_tag:?}"
TITLE="${usage_title:-Komodo Rolling Build}"
NOTES="${usage_notes:-}"
DRY_RUN="${usage_dry_run:-false}"
NOTES_FILE="$(mktemp)"
trap 'rm -f "$NOTES_FILE"' EXIT

if [ -z "$NOTES" ]; then
  {
    printf -- "- commit: \`%s\`\n" "$(git rev-parse --short HEAD)"
    printf -- '- built: %s\n' "$(date -u '+%Y-%m-%d %H:%M UTC')"
  } > "$NOTES_FILE"
else
  printf '%s\n' "$NOTES" > "$NOTES_FILE"
fi


shopt -s nullglob
ASSETS=(dist/*)
if [ "${#ASSETS[@]}" -eq 0 ]; then
  echo "error: no assets found in dist/" >&2
  exit 1
fi
if [ "$DRY_RUN" = "true" ]; then
  ACTION="$(gh release view "$TAG" >/dev/null 2>&1 && echo edit || echo create)"
  echo "tag:    $TAG"
  echo "title:  $TITLE"
  echo "notes:"
  cat "$NOTES_FILE"
  echo "assets: ${ASSETS[*]}"
  echo "plan:   gh release $ACTION + gh release upload --clobber"
  exit 0
fi

if gh release view "$TAG" >/dev/null 2>&1; then
  gh release edit "$TAG" --title "$TITLE" --notes-file "$NOTES_FILE" --latest
else
  gh release create "$TAG" --title "$TITLE" --notes-file "$NOTES_FILE" --latest
fi

gh release upload "$TAG" "${ASSETS[@]}" --clobber
