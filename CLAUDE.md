# CLAUDE.md

Guidance for Claude Code when working in this repo.

## What spyrun is

`spyrun` is a file watcher that runs commands in response to filesystem
events. Configured via TOML, with optional AES-256-GCM-SIV encryption
helpers exposed as Tera template functions for sensitive values.

See `README.md` for the user-facing schema and `example/` for sample
configs.

## Git Workflow

- **Do not push directly to the main branch.** Cut a feature branch
  and open a Pull Request.
- Exception: release-related chore commits (`chore: bump version to …`,
  `chore: release vX.Y.Z`) and pushing `git tag vX.Y.Z` may go directly
  to main, matching existing history.
- Branch names should describe the change concisely
  (`feat/...`, `fix/...`).
- **Write PR titles, bodies, and commit messages in English.**

### PR Review Cycle

- Every PR runs **Gemini Code Assist** and **CodeRabbit** reviews. Wait
  for both, address comments by pushing fixes to the PR branch, and
  reply with `@gemini-code-assist` / `@coderabbitai` after each fix so
  the bots know the feedback was acted on.
- A thread is **settled** when the latest bot reply is ack-only ("thank
  you", "acknowledged", or a re-review summary with no new findings).
  A new actionable comment unsettles it.
- **Stop conditions**: all open threads settled, OR no bot reply for
  30 min after the last actionable comment.
- **Merge gate**: review bots stopped posting actionable comments AND
  @yukimemi has approved.
- **Bot-authored PRs** (Renovate / Dependabot): review bots skip them
  by default. Merge if CI is green and the owner approves.

## Development Commands

```bash
# One-time on clone:
cargo make setup     # pre-push hook + APM install (renri skill)

# Build / test / lint
cargo make build
cargo make test
cargo make lint
cargo make check
```

`cargo make setup` is `hook-install` + `apm-install`:

- `hook-install` wires `.git/hooks/pre-push` to `cargo make check`.
- `apm-install` requires the
  [APM](https://github.com/microsoft/apm) CLI on `PATH`
  (`scoop install apm` on Windows, `brew install microsoft/apm/apm`
  on macOS, `pip install apm-cli`, or
  `curl -sSL https://aka.ms/apm-unix | sh`). It runs
  `apm install`, compiling the
  [renri](https://github.com/yukimemi/renri) skill (declared in
  `apm.yml`, pinned to `#main`) into `.claude/skills/` +
  `.gemini/skills/` + `.github/skills/` so AI sessions know how to
  manage worktrees / jj workspaces while developing spyrun. Lockfile
  is `apm.lock.yaml`. Pinned to `#main`, so `apm install --update`
  always pulls the latest renri skill content.

## Working in this repo with AI agents

- **Read-only inspection** (browsing files, answering questions,
  running read-only commands): no worktree needed; work in the
  existing checkout.
- **Any commit-bound change** — new feature, bug fix, refactor,
  reviewer-feedback fix on an open PR: if you are on the **main
  checkout**, start with `renri add <branch-name>` and move into
  the worktree before committing (`cd "$(renri cd <branch-name>)"`,
  or use the shell wrapper from `renri shell-init` so plain
  `renri cd <name>` cds for you). If you are **already in a
  worktree** (e.g. iterating on an existing PR), keep working
  there. Do **not** edit on the main checkout for non-trivial
  changes.
- **Trivial wording / typo fixes** are the only soft exception, and
  even then `renri add` is cheap enough that defaulting to it is
  fine.

### Backend choice — jj-first when available

spyrun is published as a plain git repo, but `cargo make setup` runs
`jj git init --colocate` if `jj` is on `PATH`, bringing spyrun in
line with the rest of the yukimemi/* family. Once colocated,
`renri add` defaults to **jj** (creates a non-colocated jj workspace
where `jj` commands work and `git` does not — see
[jj-vcs/jj#8052](https://github.com/jj-vcs/jj/issues/8052) for why
secondary colocation isn't possible yet).

If `jj` isn't installed, `cargo make setup` skips the colocate step,
and `renri add` falls back to `git worktree add`. The renri workflow
works either way.

```sh
# In a freshly created jj workspace (default once colocated):
jj describe -m "feat: ..."
jj git push --bookmark <branch-name> --allow-new

# In a git worktree (when jj isn't installed):
git push -u origin <branch-name>      # first push
git push                              # subsequent pushes
```

`renri --vcs git add <branch-name>` is the override and exists for
genuine git-CLI-only needs (git submodule, native git2 tooling,
git-only hooks). Do **not** reach for it out of git-CLI familiarity
— prefer learning the equivalent jj commands once colocated.

### Cleanup after merge

After the PR merges and you've pulled the change into main:

- `renri remove <branch>` — removes a single worktree. Calls
  `git worktree remove` or `jj workspace forget` as appropriate,
  then deletes the directory. Refuses to remove the main worktree.
- `renri prune` — best-effort GC across the repo. Git: removes
  worktree metadata for already-deleted directories. jj: forgets
  workspaces whose root path is gone.

Run `renri prune` periodically — especially after manually
`rm -rf`-ing worktree dirs without going through `renri remove`.

### Hooks in worktrees

The pre-push hook installed by `cargo make hook-install` lives in
the **main repo's** `.git/hooks/pre-push`.

- **git worktrees** share that hook directory, so plain `git push`
  from a worktree triggers `cargo make check` automatically.
- **jj workspaces** route their pushes through `jj git push`, which
  uses libgit2 directly and **does not fire git hooks**. From a jj
  workspace, run `cargo make check` manually before
  `jj git push --bookmark <branch-name>` — there is no automatic gate.

### Post-create automation (`cargo make on-add`)

`renri.toml` declares a `[[hooks.post_create]]` that runs
`cargo make on-add` immediately after `renri add` finishes. The
default chain is:

- `apm install --update` — refresh the renri skill so AI agents in
  the new worktree see the latest guidance.
- `vcs-fetch` — `jj git fetch` in a jj workspace, `git fetch`
  otherwise; cleans up subsequent rebase / merge.

Add per-repo extras (e.g. `cargo fetch`) by extending
`[tasks.on-add]`'s dependency list in `Makefile.toml`.
