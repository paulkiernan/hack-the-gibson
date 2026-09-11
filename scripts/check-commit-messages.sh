#!/usr/bin/env bash
#
# Conventional Commits gate for the commits a pull request adds.
#
#   scripts/check-commit-messages.sh [<rev-range>]
#
# With a rev-range every non-merge commit subject in it is checked. CI passes the two commit ids
# of the pull request as `base...head` (three dots, so the range starts at the merge base), which
# is what makes this a gate on *this branch's* commits: history before the convention was adopted
# is never judged, and a pull request therefore cannot fail on a message it did not write.
#
# With no argument, subjects are read from stdin, one per line. That is how the rules themselves
# are exercised (there is no git history involved), and how a message can be checked before it is
# committed:
#
#   echo 'fix(render): stop wgpu refusing every frame as occluded' \
#     | scripts/check-commit-messages.sh
#
# A subject is
#
#   type(scope)!: description
#
# with the scope optional (`docs: ...`) and `!` only for a breaking change (`feat(types)!: ...`).
# Only the *shape* is checked. This cannot tell `fix(render): stop the leak` from
# `fix(render): add a leak`, and it deliberately has no opinion about the body or the
# `BREAKING CHANGE:` footer beyond what `.gitmessage` says. See CONTRIBUTING.md.
#
# Merge commits are skipped: their subject is git's, not the author's. Everything else, including
# an empty message, has to pass.

set -u

ALLOWED_TYPES="feat fix docs style refactor perf test build ci chore revert"
ALLOWED_SCOPES="types scene atlas floor render core app web ffi macos linux windows ci docs"
MAX_COLUMNS=72

checked=0
rejected=0

in_list() {
    local needle="$1" list="$2" item
    for item in $list; do
        [ "$item" = "$needle" ] && return 0
    done
    return 1
}

report() {
    # report <displayed subject> <reasons, empty when the subject is good>
    local subject="$1" reasons="$2"
    checked=$((checked + 1))
    if [ -z "$reasons" ]; then
        printf 'ok    %s\n' "$subject"
        return
    fi
    rejected=$((rejected + 1))
    printf 'FAIL  %s\n' "$subject"
    printf '      %s\n' "$reasons"
}

check_subject() {
    local subject="$1"
    local reasons=""
    local head="" type="" scope=""
    local desc=""

    if [ -z "$subject" ]; then
        report "(empty subject)" "the subject is empty"
        return
    fi

    if [ "${#subject}" -gt "$MAX_COLUMNS" ]; then
        reasons="subject is ${#subject} columns; keep it to $MAX_COLUMNS or fewer"
    fi

    case "$subject" in
    *": "*) ;;
    *)
        report "$subject" "${reasons:+$reasons; }no 'type(scope): ' prefix, e.g. 'fix(render): ...'; allowed types: $ALLOWED_TYPES"
        return
        ;;
    esac

    head="${subject%%: *}"
    desc="${subject#*: }"

    # Both patterns are held in variables: an inline `=~` pattern containing parentheses is a
    # syntax error in the conditional-expression parser (checked against bash 3.2 and 5.3).
    local header_re='^([a-zA-Z]+)(\([^)]*\))?!?$'
    local scope_re='\(([^)]*)\)'

    if [[ "$head" =~ $header_re ]]; then
        type="${BASH_REMATCH[1]}"
        if ! in_list "$type" "$ALLOWED_TYPES"; then
            reasons="${reasons:+$reasons; }unknown type '$type'; allowed types: $ALLOWED_TYPES"
        fi
        if [[ "$head" =~ $scope_re ]]; then
            scope="${BASH_REMATCH[1]}"
            if ! in_list "$scope" "$ALLOWED_SCOPES"; then
                reasons="${reasons:+$reasons; }unknown scope '$scope'; allowed scopes: $ALLOWED_SCOPES (or omit the scope for a repo-wide change)"
            fi
        fi
    else
        reasons="${reasons:+$reasons; }the part before ': ' must be 'type', 'type(scope)' or 'type(scope)!'; allowed types: $ALLOWED_TYPES"
    fi

    if [ -z "$desc" ]; then
        reasons="${reasons:+$reasons; }the description is empty"
    else
        case "$desc" in
        [a-z0-9]*) ;;
        *) reasons="${reasons:+$reasons; }the description must start lower case ('${desc}')" ;;
        esac
        case "$desc" in
        *.) reasons="${reasons:+$reasons; }the description must not end with a period" ;;
        esac
    fi

    report "$subject" "$reasons"
}

if [ "$#" -gt 0 ]; then
    range="$1"
    # %s is the subject line only; --no-merges drops commits whose message is not the author's.
    while IFS= read -r subject; do
        check_subject "$subject"
    done < <(git log --no-merges --format=%s "$range")
else
    while IFS= read -r subject || [ -n "$subject" ]; do
        check_subject "$subject"
    done
fi

printf '\n%d subject(s) checked, %d rejected\n' "$checked" "$rejected"
[ "$rejected" -eq 0 ]
