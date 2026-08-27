#!/usr/bin/env bash
set -euo pipefail

manifest_path="governance/manifest.toml"

usage() {
    cat >&2 <<'USAGE'
usage:
  ./scripts/check_governance.sh working-tree
  ./scripts/check_governance.sh diff <base-commit> <head-commit>

Ordinary changes fail when they modify protected governance or human-authority paths.
A deliberate human governance review may set ALLOW_GOVERNANCE_CHANGE=1.
Implementation agents must not use that override.
USAGE
    exit 2
}

# These paths protect the guard even if a proposed change edits the manifest that
# would otherwise enumerate them. Additional authority paths are loaded from the
# trusted manifest used for the comparison.
core_patterns=(
    'governance/**'
    'AGENTS.md'
    'scripts/check_governance.sh'
    '.github/workflows/governance-integrity.yml'
    '.github/CODEOWNERS'
)

manifest_at() {
    local ref=${1:-}
    if [[ -n "$ref" ]]; then
        git show "${ref}:${manifest_path}"
    else
        cat "$manifest_path"
    fi
}

manifest_protected_patterns() {
    local ref=${1:-}
    manifest_at "$ref" | awk '
        /^\[\[protected\]\]/ { in_protected = 1; next }
        /^\[\[/ { in_protected = 0 }
        in_protected && /^[[:space:]]*path[[:space:]]*=/ {
            line = $0
            sub(/^[[:space:]]*path[[:space:]]*=[[:space:]]*"/, "", line)
            sub(/"[[:space:]]*$/, "", line)
            print line
        }
    '
}

path_matches_pattern() {
    local path=$1
    local pattern=$2
    [[ "$path" == $pattern ]]
}

check_changed_paths() {
    local manifest_ref=$1
    shift

    local -a patterns=("${core_patterns[@]}")
    local pattern
    while IFS= read -r pattern; do
        [[ -n "$pattern" ]] && patterns+=("$pattern")
    done < <(manifest_protected_patterns "$manifest_ref")

    local -a protected_changes=()
    local path
    for path in "$@"; do
        [[ -z "$path" ]] && continue
        for pattern in "${patterns[@]}"; do
            if path_matches_pattern "$path" "$pattern"; then
                protected_changes+=("$path")
                break
            fi
        done
    done

    if (( ${#protected_changes[@]} == 0 )); then
        printf 'governance integrity: no protected authority changes detected\n'
        return 0
    fi

    printf 'governance integrity: protected authority/control changes detected:\n' >&2
    printf '  %s\n' "${protected_changes[@]}" | sort -u >&2

    if [[ "${ALLOW_GOVERNANCE_CHANGE:-0}" == "1" ]]; then
        printf 'governance integrity: deliberate governance-change override present\n'
        return 0
    fi

    cat >&2 <<'EOF_BLOCK'

Ordinary implementation work must not modify these paths.
If this is a deliberate human-reviewed authority change, use the dedicated
governance-change review path. Implementation agents must not set the override themselves.
See GOV-AUTH-* and GOV-CHANGE-* in governance/policies/governance.md.
EOF_BLOCK
    return 1
}

case "${1:-}" in
    working-tree)
        (( $# == 1 )) || usage
        [[ -f "$manifest_path" ]] || {
            printf 'missing governance manifest: %s\n' "$manifest_path" >&2
            exit 1
        }
        mapfile -t changed < <(
            {
                git diff --name-only --diff-filter=ACDMRTUXB
                git diff --cached --name-only --diff-filter=ACDMRTUXB
                git ls-files --others --exclude-standard
            } | sed '/^$/d' | sort -u
        )
        check_changed_paths "" "${changed[@]}"
        ;;
    diff)
        (( $# == 3 )) || usage
        base=$2
        head=$3
        git cat-file -e "${base}^{commit}"
        git cat-file -e "${head}^{commit}"
        git cat-file -e "${base}:${manifest_path}"
        mapfile -t changed < <(git diff --name-only --diff-filter=ACDMRTUXB "$base" "$head")
        # Use the base branch's manifest, never the proposed head's manifest.
        check_changed_paths "$base" "${changed[@]}"
        ;;
    *)
        usage
        ;;
esac
