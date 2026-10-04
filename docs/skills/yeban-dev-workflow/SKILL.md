# yeban-dev-workflow

How work is done here, and how several lines of work run at once without stepping on each other.
Written from a session that produced five red builds and eleven measured lessons; no code, just the rules.

## The ten rules, each one paid for

1. **Measure before changing.** Read the real bytes, schemas, files, projects. Two "must fix" report items turned
   out to be already satisfied, and one believed field was measured to be zero in every real file — the belief was
   withdrawn, along with the criterion that had asserted it.
2. **A criterion must be able to fail.** Write it, then weaken the code and watch it go red, then restore. A check
   that has never been red is a comment wearing a test's clothes.
3. **Gate the commit on the type checker, not on your eyes.** Printing a typecheck and committing anyway shipped a
   type error. The exit code must block the commit.
4. **Never pipe a gate.** `gate | tail` reports `tail`'s exit code: a release pipeline's failure looked like success.
5. **Change a count, run the checks that derive the count.** Adding one export made a "seven formats" sentence false;
   adding tools broke a name-vs-annotation rule that the protocol gate does not cover.
6. **Find the touched set by searching, never from memory.** Ask what the change affects. The set recalled was six
   files; the truth was nine.
7. **"It applies" is not "it is needed", and "no lines matched" is not "not done".** A three-way apply succeeds on
   conflicts; a commit with 0% of its lines present can still have its intent delivered better elsewhere — including
   a 732-line rewrite of its 243-line file.
8. **Prove provenance by patch equivalence, not by the range log.** The range log counts one patch twice under two
   shas and reports work as ahead that is already in.
9. **One writer per file, and write the numbers down.** If a reading is not in the ledger with its method and moment,
   the next person re-derives it wrongly.
10. **Record your own mistakes together with the rule they produced.** Cheapest documentation there is.

## Reach outside before you invent

A design question or a thorny bug is usually **already solved somewhere**. Reach for that first.

1. **Search, then read, then design.** Look for the vendor's own specification, a maintained open-source
   implementation, or a well-argued write-up before inventing a format, an algorithm or a workaround. Reinventing
   something the field has settled is how a small problem becomes a large one.
2. **Reuse by understanding, not by pasting.** Name what the reference does and why it does it there, then write what
   *this* project needs. Code you cannot explain is code you cannot fix later.
3. **Two independent sources beat one.** Prefer a specification **and** an implementation; where they disagree, say so
   and measure — the real artefacts in front of you decide, not the louder article.
4. **Licences are part of the design.** MIT/Apache: reuse with attribution. GPL: read, learn, quote a short line —
   do not copy the code into this tree. Record the source and its licence next to whatever borrows from it.
5. **Prefer the head of the field, and say who you followed.** Quality over novelty: a maintained project with tests
   and issues beats a clever gist. Credit the project you learned from, even when no licence demands it.
6. **When nothing is found, record the search.** A `needs` entry naming what you looked for and what you found is worth
   more than an invented answer, and it tells the next person where to start.

## Per-change workflow

1. Measure; write the number down before touching anything.
2. Choose the smallest change, and state its honest boundary: what it does **not** prove.
3. Write the criterion first, prove it can go red, restore.
4. Typecheck as a gate; lint the files touched; run the **searched** family of checks, not the remembered one.
5. Commit with a message that says what is *not* done; push; then **read the verdict**.
6. Red: read the gate's own words, fix, repeat. Green: record commit + verdict in the ledger.
7. Anything unproven goes into a `needs` list. Never silently degrade.

## Fast multi-line iteration

- **One worktree per line of work**, one branch each. Two lines never share a tree.
- Lines stay **short-lived** and land through the same gates; a blocked line must not block the others.
- Each line runs the **cheap gate set** (typecheck, its touched family, the protocol gate) rather than the full suite.
- **Land small, land often.** The smaller the commit, the smaller the red and the shorter the hunt.
- A superseded line is **abandoned explicitly**: tag it, bundle it, delete the branch. A stale branch is a liability,
  not a backup.
- Before believing a line is still needed, measure: patch equivalence plus content overlap.
- Never let two agents write one file. Merging is where the speed goes.

## Before deleting anything

Preserve first (tag + bundle), then delete, then **read back** what remains. Keep the minimum needed to keep working —
the working directory and the ledger — and say which you kept and why.

## Honesty rules

Local green is not green; a verdict not yet read is `pending`. "It opens in the DAW" needs a machine with the DAW.
Say the unverified half out loud, and put the rest in `needs`.
