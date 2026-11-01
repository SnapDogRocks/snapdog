#!/usr/bin/env python3
"""Check that a pull request carries the contribution licence declaration.

Usage: check-contribution-license.py PR.json COMMITS.json

PR.json      the pull request object of the GitHub API
COMMITS.json the list of commits of the pull request (GitHub API, all pages merged)

Environment: MAINTAINERS, comma separated GitHub logins that own the project and need no declaration.

Rules (see CONTRIBUTION-LICENSE.md, section 8):
  * Bots and maintainers pass.
  * The pull request description contains the ticked box with the text CONTRIBUTION-LICENSE v1.
  * Every commit by another person than the pull request author, and every Co-authored-by person,
    is covered by a Signed-off-by line with that person's email address in the same commit.
Standard library only. Exit status 1 on a missing declaration.
"""
import json
import os
import re
import sys

MARKER = "CONTRIBUTION-LICENSE v1"
BOX = re.compile(r"^\s*[-*]\s*\[[xX]\]\s.*" + re.escape(MARKER), re.M)
SIGNED = re.compile(r"^Signed-off-by:\s*.*?<([^<>\s]+)>\s*$", re.M | re.I)
COAUTHOR = re.compile(r"^Co-authored-by:\s*(.*?)\s*<([^<>\s]+)>\s*$", re.M | re.I)


def is_bot(login, kind=None):
    return kind == "Bot" or bool(login and login.endswith("[bot]"))


def check(pr, commits, maintainers):
    author = (pr.get("user") or {}).get("login")
    if is_bot(author, (pr.get("user") or {}).get("type")) or author in maintainers:
        return []
    problems = []
    if not BOX.search(pr.get("body") or ""):
        problems.append("The pull request description lacks the ticked box '%s'. Tick it in the template or add:\n"
                        "- [x] I have read CONTRIBUTION-LICENSE.md (%s) and license this contribution under its terms."
                        % (MARKER, MARKER))
    for c in commits:
        login = (c.get("author") or {}).get("login")
        msg = (c.get("commit") or {}).get("message") or ""
        email = ((c.get("commit") or {}).get("author") or {}).get("email") or ""
        signed = {m.lower() for m in SIGNED.findall(msg)}
        short = c.get("sha", "")[:7]
        if login != author and not is_bot(login) and login not in maintainers and email.lower() not in signed:
            problems.append("Commit %s is by %s, who is not the pull request author: add 'Signed-off-by: Name <%s>'."
                            % (short, login or email or "an unknown author", email))
        for name, mail in COAUTHOR.findall(msg):
            if "[bot]" in name.lower() or mail.lower() in signed:
                continue
            problems.append("Commit %s names %s as co-author: add 'Signed-off-by: %s <%s>'." % (short, name, name, mail))
    return problems


def main(argv):
    if len(argv) != 3:
        sys.exit(__doc__)
    with open(argv[1], encoding="utf-8") as f:
        pr = json.load(f)
    with open(argv[2], encoding="utf-8") as f:
        commits = json.load(f)
    maintainers = {m.strip() for m in os.environ.get("MAINTAINERS", "").split(",") if m.strip()}
    problems = check(pr, commits, maintainers)
    for p in problems:
        print("::error::" + p.replace("\n", "%0A"))
    if problems:
        print("See CONTRIBUTION-LICENSE.md.")
        return 1
    print("Contribution licence declaration present.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
