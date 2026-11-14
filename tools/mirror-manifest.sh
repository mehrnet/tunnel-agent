#!/bin/sh
# Convert a verified relative-parts manifest into a direct-URL mirror manifest.
set -eu

: "${SOURCE_MANIFEST:?set SOURCE_MANIFEST to the canonical .parts file}"
: "${PART_URLS:?set PART_URLS to ordered comma-separated HTTPS part URLs}"

if [ ! -f "$SOURCE_MANIFEST" ]; then
  echo "source manifest does not exist: $SOURCE_MANIFEST" >&2
  exit 2
fi

urls_file=$(mktemp)
trap 'rm -f "$urls_file"' EXIT HUP INT TERM
printf '%s\n' "$PART_URLS" | tr ',' '\n' > "$urls_file"

read_url() {
  url=$(sed -n "$1p" "$urls_file")
  case "$url" in
    https://*) ;;
    *) echo "part URL $1 must use HTTPS" >&2; exit 2 ;;
  esac
  case "$url" in *[[:space:],\\]*|*'#'*) echo "part URL $1 contains unsafe characters" >&2; exit 2;; esac
  printf '%s\n' "$url"
}

part_index=1
first_line=true
while IFS= read -r line || [ -n "$line" ]; do
  if [ "$first_line" = true ]; then
    [ "$line" = 'TUNNEL_AGENT_BUNDLE_PARTS=1' ] || { echo "source must be a v1 manifest" >&2; exit 2; }
    printf '%s\n' 'TUNNEL_AGENT_BUNDLE_PARTS=2'
    first_line=false
    continue
  fi
  case "$line" in
    PART\ *)
      fields=${line#PART }
      part_sha=${fields%% *}
      fields=${fields#* }
      part_bytes=${fields%% *}
      printf 'PART %s %s %s\n' "$part_sha" "$part_bytes" "$(read_url "$part_index")"
      part_index=$((part_index + 1))
      ;;
    *) printf '%s\n' "$line" ;;
  esac
done < "$SOURCE_MANIFEST"

if [ "$(wc -l < "$urls_file" | tr -d '[:space:]')" -ne $((part_index - 1)) ]; then
  echo "PART_URLS count does not match source manifest" >&2
  exit 2
fi
