---
name: rain-review
description: The full review and release cycle for Rain - complete checklist, multi-agent code review of everything since the last review, fixes, release install and a check in the real window. Run only when the owner types /rain-review; never start it on your own.
disable-model-invocation: true
---

# /rain-review

The heavy cycle for Rain. Everyday changes skip all of this (see CLAUDE.md, "the fast loop").
The owner typed `/rain-review`, so the multi-agent work below is asked for. Arguments, if any,
narrow the scope (a path, a feature name, or a commit range).

## 1. Scope

- The last reviewed commit is the local git tag `rain-reviewed`. Without the tag, use
  `origin/main`.
- The scope is `git diff rain-reviewed..HEAD` plus uncommitted changes. List the changed areas
  (backend modules, screens, scripts) in one short paragraph before starting.
- Commit uncommitted work first, so every fix can be seen as its own diff.

## 2. Full checklist

Run `npm run check` in the background (it takes several minutes). Stop the dev backend first
(`npm run backend:stop`) so that the build does not wait on a locked program file. A failure of
a timing test under load is run again alone before it counts.

## 3. Review, in parallel

Use the Workflow tool with Sonnet agents, one finder per area of the scope, each with these
lenses:

1. Correctness: wrong results, races, error paths, a state that cannot be left.
2. The rules of CLAUDE.md: the vault and the data folder, `~/.claude` read only, game mode never
   closing a real program in a test or dev run, `unsafe` only in `platform/`, nothing running
   while Rain is hidden.
3. The screen against `docs/ui-spec.md`: keys, empty and error states, the shortcut sheet.
4. Simplicity: unused code, a second way to do what one place already does.

Every finding is then checked by an independent agent that tries to refute it (default: refuted
when unsure). Only confirmed findings go on. Agents never read the real vault or the real data
folder and never run a dev server.

## 4. Fix

Fix the confirmed findings in the main tree, one commit per finding or per small group. Run
`npm run check:quick` after each group and the full `npm run check` once at the end.

## 5. Install and look

- `npm run install:local`, then restart the installed Rain.
- Check in the real window what the scope touched (start `rain.exe` with
  `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` and drive it over the
  DevTools protocol, so the owner's screen is not taken over).
- `cargo clean` afterwards if `target` grew past 20 GB.

## 6. Report

- What was reviewed, what was found and fixed, what was left and why, what was not verified.
- Move the tag: `git tag -f rain-reviewed HEAD`.
- Do not push. Offer it.
