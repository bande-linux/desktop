You are refreshing the patches of Bande's Browser for a new Debian firefox-esr upload. Our patches
apply after Debian's own, and at least one no longer applies.

State: Debian's source is unpacked with Debian's patches applied, and ours are appended to
`debian/patches/series` under `bande/`. `quilt` (with `QUILT_PATCHES=debian/patches`) stopped at the
first of ours that fails; `quilt next` names it.

Do this, in the source directory, with `export QUILT_PATCHES=debian/patches QUILT_PC=.pc`:
1. For each of our patches that fails, `quilt push -f` it, resolve every rejected hunk by hand so
   the patch keeps its intent on the new code, delete the `.rej` files, and `quilt refresh`. Keep its
   description header. Continue with `quilt push --fuzz=0` until `quilt push -a --fuzz=0` reports the
   series fully applied.
2. Only touch our patches. Never change Debian's patches or anything outside the source tree and our
   series directory.
3. Copy every refreshed patch back over the file of the same name in our series directory, keeping
   its `series` file as it is.
4. If the code a patch targets was rewritten so much that the change no longer makes sense, or it
   would remove less than before (anything that sends data must stay removed), say so plainly and
   stop without changing that patch: a person decides then.

Finish with a short report: each patch you refreshed and what you had to change.
