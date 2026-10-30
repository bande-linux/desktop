#!/bin/bash
# Keeps one open issue while main is red: opens or updates it when a main run fails and closes
# it once a run passes. Needs GH_TOKEN with issues: write, GITHUB_REPOSITORY, GITHUB_RUN_ID,
# GITHUB_SHA and GITHUB_SERVER_URL.
set -euo pipefail

title="CI is red on main"
api=https://api.github.com/repos/$GITHUB_REPOSITORY
run_url=$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID

github() {
    local method=$1 path=$2
    shift 2
    curl -fsS -X "$method" -H "Authorization: Bearer $GH_TOKEN" \
        -H 'Accept: application/vnd.github+json' "$api$path" "$@"
}

issue=$(github GET "/issues?state=open&per_page=100" \
    | jq --arg title "$title" 'map(select(.title == $title and .pull_request == null)) | first | .number // empty')
failed=$(github GET "/actions/runs/$GITHUB_RUN_ID/jobs?per_page=100" \
    | jq -r '.jobs[] | select(.conclusion == "failure" or .conclusion == "timed_out" or .conclusion == "cancelled") | "- [\(.name)](\(.html_url))"')

if [ -z "$failed" ]; then
    if [ -n "$issue" ]; then
        jq -n --arg body "Green again in $run_url (${GITHUB_SHA:0:7})." '{body: $body}' \
            | github POST "/issues/$issue/comments" --data @- > /dev/null
        github PATCH "/issues/$issue" --data '{"state": "closed", "state_reason": "completed"}' > /dev/null
        echo "Closed #$issue"
    fi
    exit 0
fi

body="The [main run]($run_url) for ${GITHUB_SHA:0:7} failed in:

$failed"
if [ -n "$issue" ]; then
    jq -n --arg body "$body" '{body: $body}' | github POST "/issues/$issue/comments" --data @- > /dev/null
    jq -n --arg body "$body" '{body: $body}' | github PATCH "/issues/$issue" --data @- > /dev/null
    echo "Updated #$issue"
else
    jq -n --arg title "$title" --arg body "$body" '{title: $title, body: $body}' \
        | github POST /issues --data @- | jq -r '"Opened #\(.number)"'
fi
