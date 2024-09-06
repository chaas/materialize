import argparse
import sys

from materialize import MZ_ROOT, spawn
from materialize.git import checkout, get_branch_name, tag_annotated# Copyright Materialize, Inc. and contributors. All rights reserved.
#
# Use of this software is governed by the Business Source License
# included in the LICENSE file at the root of this repository.
#
# As of the Change Date specified in that file, in accordance with
# the Business Source License, use of this software will be governed
# by the Apache License, Version 2.0.

"""Create release notes file based on the commit history."""

import argparse
import sys
import re

from materialize.mz_version import MzVersion
from materialize.git import get_commits_between_versions

def extract_release_notes_from_commit(commit):
    # Define a regex pattern to match everything after "### Release Note" until the next "###" or EOF
    pattern = re.compile(r"### Release Note\s*(.*?)(\n###|\Z)", re.DOTALL)
    match = pattern.search(commit)
    if match:
        release_notes = match.group(1).strip()
        return release_notes
    else:
        return None
    
def extract_pr_id_from_commit(commit):
    pattern = re.compile(r"#[0-9]{1,10}", re.DOTALL)
    match = pattern.search(commit)
    if match:
        pr_id = match.group(0).strip()
        return pr_id
    else:
        return None

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--release_version")
    parser.add_argument("--previous_version")
    parser.add_argument("--date")
    args = parser.parse_args()

    release_version = MzVersion.parse_mz(args.release_version)
    previous_version = MzVersion.parse_mz(args.previous_version)

    commits = get_commits_between_versions(previous_version, release_version)
    
    release_notes = []
    for commit in commits:
        release_note = extract_release_notes_from_commit(commit)
        if not release_note:
            continue
        pr_id = extract_pr_id_from_commit(commit)
        row = f"  - {release_note}"
        if pr_id:
            row += f" (#{pr_id})"
        release_notes.append(row)
    
    release_note_str = f"""---
# Release notes for {release_version} - {args.date}

"""
    if release_notes:
        for note in release_notes:
            release_note_str += note + "\n"
    
    else:
        release_note_str += "No release notes for this version."
        
    print(release_note_str)

if __name__ == "__main__":
    sys.exit(main())
