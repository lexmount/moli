# Syncing

This file documents the process of syncing this repository with the upstream copy of Stylo in mozilla-central.

## Syncing `upstream` with mozilla-central

Start by generating a filtered copy of mozilla-central. This will cache the raw mozilla-central in `_cache/upstream`, storing the result in `_filtered`:

```sh
$ ./sync.sh _filtered
```

If `_filtered` already exists, you will need to delete it and try again:

```sh
$ rm -Rf _filtered
```

Now overwrite our `upstream` with those commits and push:

```sh
$ git fetch -f --progress ./_filtered main:upstream
$ git push -fu --progress origin upstream
```

## Rebasing `main` onto `upstream`

Start by fetching `upstream` into your local repo:

```sh
$ git fetch -f origin upstream:upstream
```

In general, the filtering process is deterministic, yielding the same commit hashes each time, so we can rebase normally:

```sh
$ git rebase upstream
```

But if the filtering config changes or Mozilla moves to GitHub, the commit hashes on `upstream` may change. In this case, we need to tell git where the old upstream ends and our own commits start (notice the `~`):

```sh
$ git log --pretty=\%H --grep='Servo initial downstream commit'
e62d7f0090941496e392e1dc91df103a38e3f488

$ git rebase --onto upstream e62d7f0090941496e392e1dc91df103a38e3f488~
Successfully rebased and updated refs/heads/main.
```

`start-rebase.sh` takes care of this automatically, but you should still use `git rebase` for subsequent steps like `--continue` and `--abort`:

```sh
$ ./start-rebase.sh upstream
$ ./start-rebase.sh upstream -i     # interactive
$ git rebase --continue             # not ./start-rebase.sh --continue
$ git rebase --abort                # not ./start-rebase.sh --abort
```

Or if we aren’t ready to rebase onto the tip of upstream:

```sh
$ ./start-rebase.sh upstream~10 -i
```
