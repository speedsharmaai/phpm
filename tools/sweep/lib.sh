# shellcheck shell=bash disable=SC2034
# jq programs shared by build-corpus and its tests.

# The fields every repository lookup asks for.
REPO_FIELDS='nameWithOwner stargazerCount isArchived isFork isPrivate
  defaultBranchRef { target { oid } }
  lock: object(expression: "HEAD:composer.lock") { ... on Blob { byteSize } }
  json: object(expression: "HEAD:composer.json") { ... on Blob { byteSize } }'

# A repository node (plus $source, $taken, optional $fixture and $pin) to a
# corpus entry, or to {dropped: reason} when the sweep cannot use it.
# shellcheck disable=SC2016
ENTRY_JQ='
  def why:
    if . == null then "not found"
    elif .isPrivate then "private"
    elif .isArchived then "archived"
    elif .isFork then "fork"
    elif .json == null then "no root composer.json"
    elif .lock == null and ($fixture // "") == "" then "no root composer.lock at the default branch"
    else null end;
  (why) as $why
  | if $why != null then {repo: (.nameWithOwner // $repo), dropped: $why}
    else {
      repo: .nameWithOwner,
      commit: (if ($pin // "") != "" then $pin else .defaultBranchRef.target.oid end),
      stars: .stargazerCount,
      taken: $taken,
      source: $source,
      notes: (if ($fixture // "") != "" then "no committed lock; installs fixtures/\($fixture) in this repository" else "" end)
    } + (if ($fixture // "") != "" then {fixture: $fixture} else {} end)
    end'

# Hand-picked entries first, then search results by stars, one entry per
# repository (case-insensitive), cut to $target projects.
# shellcheck disable=SC2016
MERGE_JQ='
  def key: (.repo // ("fixture:" + .fixture)) | ascii_downcase;
  (.[0] | map(select(.dropped == null))) as $picked
  | ($picked | map(key)) as $taken_keys
  | (.[1] | map(select(.dropped == null))
     | map(select(key as $k | $taken_keys | index($k) | not))
     | unique_by(key)
     | sort_by(-.stars, (.repo | ascii_downcase))) as $found
  | ($target - ($picked | length)) as $room
  | {
      taken: $taken,
      projects: (($picked + $found[:([$room, 0] | max)])
        | sort_by((if .stars == null then 1 else 0 end), -(.stars // 0), key)),
      dropped: (.[0] | map(select(.dropped != null)))
    }'
