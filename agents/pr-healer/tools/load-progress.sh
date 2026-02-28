#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PROGRESS_FILE="$REPO_ROOT/agents/pr-healer/progress.jsonl"

if [[ ! -f "$PROGRESS_FILE" ]]; then
    echo "📋 No progress file found - this is a fresh start"
    exit 0
fi

echo "📋 Progress History:"
echo "===================="

# Group by PR and show latest status
{
    read -r header
    echo "$header"
    cat "$PROGRESS_FILE"
} < <(printf "PR\tBranch\tTimestamp\tStatus\n") | column -t -s $'\t'

echo ""
echo "Summary:"
echo "--------"

# Count unique PRs processed
UNIQUE_PRS=$(jq -r '[.[] | .pr] | unique | length' "$PROGRESS_FILE" 2>/dev/null || echo "0")
echo "Unique PRs processed: $UNIQUE_PRS"

# Show recent attempts
echo ""
echo "Recent attempts (last 10):"
tail -n 10 "$PROGRESS_FILE" 2>/dev/null | while IFS=$'\t' read -r pr branch timestamp status; do
    echo "  PR #$pr ($branch): $status at $timestamp"
done

echo ""
echo "Use this information to:"
echo "  - Skip PRs that have already been processed successfully"
echo "  - Retry PRs that failed previously"
echo "  - Avoid repeating the same fixes"
