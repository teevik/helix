#!/bin/bash

set -e
set -x

# This script assumes CWD is the Helix repo. Run it at the repo root.

# The list of PRs to pick here:
INTERESTING_PRS=(
  # Add optional substring matching for picker
  5114
  # Fix old values shown in `select_register`
  5242
  # Make search commands respect register selection
  5244
  # Support going to specific positions in file
  5260
  # Only render the auto-complete menu if it intersects with signature help
  5523
  # Changed file picker
  5645
  # search buffer
  5652 
  # duplicate symlink
  5658
  # inital highligh sort order
  5196
  # # rainbow
  # 2857
  # rework positioning/rendering and enable softwrap/virtual text
  5420
)

# Makes the latest PR head available at a local branch
function fetch_pr() {
  PR="$1"

  git branch -D pr/$PR || true
  git fetch upstream refs/pull/$PR/head:pr/$PR
}

# Squashs the PR into the local `batteries` branch
function add_pr() {
  PR="$1"

  git branch -D temp || true
  git checkout -b temp

  git reset --hard pr/$PR
  git rebase batteries --no-gpg-sign

  git reset batteries
  git add .

  # We don't add the "#" before the PR number to avoid spamming the PR thread
  git commit -m "PR $PR" --no-gpg-sign

  git checkout batteries
  git reset --hard temp

  git branch -D temp
}

git fetch upstream
git checkout batteries
git reset --hard upstream/master

# Updates the PRs first so that we still have latest heads even if rebase fails
# for PR in ${INTERESTING_PRS[@]}; do
#   fetch_pr $PR
# done

# Actual rebasing and squashing
for PR in ${INTERESTING_PRS[@]}; do
  add_pr $PR
done

# Additional local stuff here
# git cherry-pick ..dev/abc
# git cherry-pick ..dev/def

# Install the branch with this command
