#!/bin/bash

set -e
set -x

# This script assumes CWD is the Helix repo. Run it at the repo root.

# The list of PRs to pick here:
INTERESTING_PRS=(
 # jump mode
  5340
  # brackets
  7242
  # long lived diagnostics
  6447
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
