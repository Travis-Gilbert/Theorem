#!/bin/bash
#
# ship-testflight.sh: archive the Theorem iOS app and upload it to internal
# TestFlight. This is the one-command finish to the path documented in
# docs/plans/theorem-ios-v1/native-app-shipping.md.
#
# It does NOT hide the credential wall: signing a Release archive requires an
# Apple Developer Team, and uploading requires an authenticated session. This
# script fails loudly and early if either is missing, and tells you exactly
# what to provide.
#
# Required:
#   THEOREM_TEAM_ID      Your 10-character Apple Developer Team ID
#                        (developer.apple.com -> Membership -> Team ID).
#
# Credential path (pick ONE):
#   A) Xcode account session: sign in once at
#        Xcode -> Settings -> Accounts -> (+) -> Apple ID
#      then run this script with just THEOREM_TEAM_ID set.
#   B) App Store Connect API key (headless, no GUI), set all three:
#        THEOREM_ASC_KEY_ID       the key's Key ID
#        THEOREM_ASC_ISSUER_ID    the issuer ID (ASC -> Users and Access -> Integrations)
#        THEOREM_ASC_KEY_PATH     path to the downloaded AuthKey_XXXX.p8
#
# One-time prerequisite (either path): the app record for the bundle id
#   me.travisgilbert.theorem
# must exist in App Store Connect (My Apps -> (+) -> New App) before the first
# upload, or the upload is rejected.
#
# Flags:
#   -a   archive only (validate signing; do not export or upload)
#   -h   show this help

set -euo pipefail
IFS=$'\n\t'

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT="$SCRIPT_DIR/Theorem.xcodeproj"
readonly SCHEME="Theorem"
readonly BUNDLE_ID="me.travisgilbert.theorem"
readonly EXPORT_OPTIONS="$SCRIPT_DIR/ExportOptions.plist"
readonly ARCHIVE_PATH="/tmp/Theorem.xcarchive"
readonly EXPORT_PATH="/tmp/Theorem-export"

archive_only=false

log() { echo "[ship-testflight] $*" >&2; }
die() { echo "[ship-testflight] ERROR: $*" >&2; exit 1; }

usage() {
    sed -n '2,40p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit "${1:-0}"
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

main() {
    while getopts "ah" opt; do
        case "$opt" in
            a) archive_only=true ;;
            h) usage 0 ;;
            *) usage 1 ;;
        esac
    done

    require_cmd xcodebuild
    require_cmd xcodegen

    # The Team ID is the hard precondition for any signed archive.
    local team_id="${THEOREM_TEAM_ID:-}"
    [[ -n "$team_id" ]] || die "THEOREM_TEAM_ID is unset. Set it to your Apple Developer Team ID (developer.apple.com -> Membership). Without it, 'Signing for Theorem requires a development team' and the archive cannot proceed."

    # Optional App Store Connect API key (credential path B). If all three are
    # present we authenticate headlessly; otherwise we rely on a signed-in Xcode
    # account (credential path A).
    local -a auth_args=()
    if [[ -n "${THEOREM_ASC_KEY_ID:-}" || -n "${THEOREM_ASC_ISSUER_ID:-}" || -n "${THEOREM_ASC_KEY_PATH:-}" ]]; then
        [[ -n "${THEOREM_ASC_KEY_ID:-}" ]] || die "THEOREM_ASC_KEY_ID is set partially: also set THEOREM_ASC_ISSUER_ID and THEOREM_ASC_KEY_PATH (or unset all three to use the Xcode account session)."
        [[ -n "${THEOREM_ASC_ISSUER_ID:-}" ]] || die "THEOREM_ASC_ISSUER_ID is required when using an App Store Connect API key."
        [[ -n "${THEOREM_ASC_KEY_PATH:-}" ]] || die "THEOREM_ASC_KEY_PATH is required when using an App Store Connect API key."
        [[ -f "${THEOREM_ASC_KEY_PATH}" ]] || die "THEOREM_ASC_KEY_PATH does not point to a file: ${THEOREM_ASC_KEY_PATH}"
        auth_args=(
            -authenticationKeyID "$THEOREM_ASC_KEY_ID"
            -authenticationKeyIssuerID "$THEOREM_ASC_ISSUER_ID"
            -authenticationKeyPath "$THEOREM_ASC_KEY_PATH"
        )
        log "Auth: App Store Connect API key (headless)."
    else
        log "Auth: relying on a signed-in Xcode account (Xcode -> Settings -> Accounts). Set THEOREM_ASC_* for headless auth instead."
    fi

    log "Team: $team_id"
    log "Bundle: $BUNDLE_ID"

    # Regenerate the project from the tracked spec (the .xcodeproj is gitignored).
    log "Regenerating Theorem.xcodeproj from project.yml ..."
    ( cd "$SCRIPT_DIR" && xcodegen generate --spec project.yml >/dev/null )

    log "Archiving (Release) ..."
    rm -rf "$ARCHIVE_PATH"
    xcodebuild \
        -project "$PROJECT" \
        -scheme "$SCHEME" \
        -configuration Release \
        -destination 'generic/platform=iOS' \
        -archivePath "$ARCHIVE_PATH" \
        -allowProvisioningUpdates \
        "${auth_args[@]}" \
        DEVELOPMENT_TEAM="$team_id" \
        archive
    log "Archive succeeded: $ARCHIVE_PATH"

    if [[ "$archive_only" == true ]]; then
        log "Archive-only mode: stopping before export/upload."
        exit 0
    fi

    log "Exporting and uploading to internal TestFlight via ExportOptions.plist ..."
    rm -rf "$EXPORT_PATH"
    xcodebuild \
        -exportArchive \
        -archivePath "$ARCHIVE_PATH" \
        -exportPath "$EXPORT_PATH" \
        -exportOptionsPlist "$EXPORT_OPTIONS" \
        -allowProvisioningUpdates \
        "${auth_args[@]}"

    log "Upload submitted. The build will appear in App Store Connect -> TestFlight"
    log "after processing. If the upload was rejected with 'no app record', create"
    log "the app for $BUNDLE_ID in App Store Connect (My Apps -> +) and re-run."
}

main "$@"
